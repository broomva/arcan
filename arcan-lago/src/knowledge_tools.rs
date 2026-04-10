//! Knowledge wiki tools for Arcan agents.
//!
//! Exposes `lago-knowledge` capabilities as agent tools so the LLM can
//! self-direct knowledge graph queries during its reasoning loop.

use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use lago_knowledge::bm25::Bm25Index;
use lago_knowledge::{HybridSearchConfig, KnowledgeIndex};
use serde_json::json;
use std::sync::{Arc, RwLock};
use std::time::Instant;

fn tool_err(msg: impl Into<String>) -> CoreError {
    CoreError::ToolExecution {
        tool_name: "wiki".to_string(),
        message: msg.into(),
    }
}

/// Tool that searches the knowledge graph using hybrid BM25 + graph proximity.
pub struct WikiSearchTool {
    index: Arc<RwLock<KnowledgeIndex>>,
}

impl WikiSearchTool {
    pub fn new(index: Arc<RwLock<KnowledgeIndex>>) -> Self {
        Self { index }
    }
}

impl Tool for WikiSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "wiki_search".to_string(),
            description: "Search the knowledge graph using hybrid BM25 + graph proximity scoring. Returns ranked notes with excerpts and relevance scores.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query — concepts, topics, or questions to find in the knowledge graph"
                    },
                    "max_results": {
                        "type": "integer",
                        "description": "Maximum number of results (default: 5)"
                    }
                },
                "required": ["query"]
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("knowledge".to_string()),
            tags: vec!["knowledge".to_string(), "search".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let started = Instant::now();
        let query = call
            .input
            .get("query")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| tool_err("missing 'query' field"))?;

        let max_results = call
            .input
            .get("max_results")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize)
            .unwrap_or(5);
        let span = life_vigil::spans::knowledge_search_span(query);
        let _guard = span.enter();

        let index = self
            .index
            .read()
            .map_err(|e| tool_err(format!("index lock: {e}")))?;

        let config = HybridSearchConfig {
            max_results,
            ..Default::default()
        };
        let bm25 = Bm25Index::build_with_params(index.notes(), config.bm25_k1, config.bm25_b);

        let results = index.search_hybrid(query, &bm25, &config);
        let top_relevance = results.first().map(|result| result.score).unwrap_or(0.0);

        let mut text = String::new();
        if results.is_empty() {
            text.push_str("No results found.");
        } else {
            for (i, r) in results.iter().enumerate() {
                text.push_str(&format!(
                    "{}. **{}** [score: {:.2}]\n   {}\n",
                    i + 1,
                    r.name,
                    r.score,
                    r.path
                ));
                for excerpt in r.excerpts.iter().take(2) {
                    text.push_str(&format!("   > {excerpt}\n"));
                }
                if !r.links.is_empty() {
                    text.push_str(&format!("   links: {}\n", r.links.join(", ")));
                }
                text.push('\n');
            }
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        let context_tokens = (text.len() / 4) as u32;
        life_vigil::spans::record_knowledge_search(
            &span,
            results.len() as u32,
            top_relevance,
            duration_ms,
            context_tokens,
        );

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "query": query,
                "results": text,
                "count": results.len(),
                "top_relevance": top_relevance,
                "duration_ms": duration_ms,
                "context_tokens": context_tokens,
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

/// Tool that lints the knowledge graph and reports health.
pub struct WikiLintTool {
    index: Arc<RwLock<KnowledgeIndex>>,
}

impl WikiLintTool {
    pub fn new(index: Arc<RwLock<KnowledgeIndex>>) -> Self {
        Self { index }
    }
}

impl Tool for WikiLintTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "wiki_lint".to_string(),
            description: "Check the knowledge graph health. Reports orphan pages, broken links, contradictions, stale claims, and missing pages with an aggregate health score.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: Some("knowledge".to_string()),
            tags: vec!["knowledge".to_string(), "lint".to_string()],
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let span = life_vigil::spans::knowledge_lint_span();
        let _guard = span.enter();
        let index = self
            .index
            .read()
            .map_err(|e| tool_err(format!("index lock: {e}")))?;

        let report = index.lint();
        let note_count = index.len();
        life_vigil::spans::record_knowledge_lint(
            &span,
            report.health_score.into(),
            note_count as u32,
            report.contradictions.len() as u32,
            report.missing_pages.len() as u32,
        );

        let mut text = format!(
            "## Knowledge Lint Report\n\nHealth: **{:.0}%** | Notes: {note_count}\n\n",
            report.health_score * 100.0,
        );

        if !report.orphan_pages.is_empty() {
            text.push_str(&format!(
                "**Orphans** ({}): {}\n\n",
                report.orphan_pages.len(),
                report
                    .orphan_pages
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        if !report.broken_links.is_empty() {
            text.push_str(&format!(
                "**Broken links** ({}): {}\n\n",
                report.broken_links.len(),
                report
                    .broken_links
                    .iter()
                    .take(5)
                    .map(|(s, t)| format!("{s}\u{2192}{t}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        if !report.contradictions.is_empty() {
            text.push_str(&format!(
                "**Contradictions** ({})\n\n",
                report.contradictions.len()
            ));
        }

        if !report.missing_pages.is_empty() {
            text.push_str(&format!(
                "**Missing pages** ({}): {}\n\n",
                report.missing_pages.len(),
                report
                    .missing_pages
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        if report.orphan_pages.is_empty()
            && report.broken_links.is_empty()
            && report.contradictions.is_empty()
            && report.missing_pages.is_empty()
            && report.stale_claims.is_empty()
        {
            text.push_str("No issues found.\n");
        }

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({
                "health_score": report.health_score,
                "note_count": note_count,
                "orphans": report.orphan_pages.len(),
                "broken_links": report.broken_links.len(),
                "contradictions": report.contradictions.len(),
                "missing_pages": report.missing_pages.len(),
                "report": text,
            }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}
