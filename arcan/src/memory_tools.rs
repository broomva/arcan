//! Agent-driven memory retrieval tools (BRO-417).
//!
//! Six tools the agent calls proactively to manage its own memory:
//! - `memory_search` — keyword search across memory files
//! - `memory_browse` — list memories by tier/type
//! - `memory_recent` — last N memories by modification time
//! - `memory_offload` — save content to episodic memory
//! - `memory_forget` — mark a memory as low importance
//! - `memory_similar` — semantic retrieval over Lance embeddings

use aios_protocol::tool::{
    Tool, ToolAnnotations, ToolCall, ToolContext, ToolDefinition, ToolError, ToolResult,
};
use lago_core::event::{EventEnvelope, EventPayload};
use lago_lance::{EMBEDDING_META_KEY, LanceJournal};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ── MemorySearchTool ────────────────────────────────────────────────

/// Keyword search across all memory files in `.arcan/memory/`.
pub struct MemorySearchTool {
    memory_dir: PathBuf,
}

impl MemorySearchTool {
    pub fn new(memory_dir: &Path) -> Self {
        Self {
            memory_dir: memory_dir.to_path_buf(),
        }
    }
}

fn parse_query_keywords(query: &str) -> Result<Vec<String>, ToolError> {
    let keywords: Vec<String> = query
        .split_whitespace()
        .map(|kw| kw.trim().to_lowercase())
        .filter(|kw| !kw.is_empty())
        .collect();

    if keywords.is_empty() {
        Err(ToolError::InvalidInput {
            message: "Query cannot be empty".into(),
        })
    } else {
        Ok(keywords)
    }
}

fn keyword_search_results(memory_dir: &Path, keywords: &[String]) -> Vec<serde_json::Value> {
    let keyword_refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
    let mut matches = Vec::new();

    if let Ok(entries) = fs::read_dir(memory_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "md")
                && let Ok(content) = fs::read_to_string(&path)
            {
                let content_lower = content.to_lowercase();
                let hit_count = keyword_refs
                    .iter()
                    .filter(|kw| content_lower.contains(*kw))
                    .count();

                if hit_count > 0 {
                    let file_name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    matches.push(json!({
                        "file": file_name,
                        "relevance": hit_count,
                        "excerpt": extract_excerpt(&content, &keyword_refs, 3),
                        "backend": "keyword",
                    }));
                }
            }
        }
    }

    matches.sort_by(|a, b| {
        b.get("relevance")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            .cmp(
                &a.get("relevance")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
            )
    });

    matches
}

fn run_async<T, F>(future: F) -> Result<T, ToolError>
where
    F: std::future::Future<Output = Result<T, lago_core::LagoError>>,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => handle
            .block_on(future)
            .map_err(|e| ToolError::ExecutionFailed {
                tool_name: "memory_similar".into(),
                message: format!("Semantic search failed: {e}"),
            }),
        Err(_) => match tokio::runtime::Runtime::new() {
            Ok(rt) => rt.block_on(future).map_err(|e| ToolError::ExecutionFailed {
                tool_name: "memory_similar".into(),
                message: format!("Semantic search failed: {e}"),
            }),
            Err(e) => Err(ToolError::ExecutionFailed {
                tool_name: "memory_similar".into(),
                message: format!("Failed to create async runtime: {e}"),
            }),
        },
    }
}

fn event_memory_content(event: &EventEnvelope) -> Option<&str> {
    match &event.payload {
        EventPayload::Message { content, .. } => Some(content.as_str()),
        _ => None,
    }
}

fn event_embedding(event: &EventEnvelope) -> Option<Vec<f32>> {
    event
        .metadata
        .get(EMBEDDING_META_KEY)
        .and_then(|json| serde_json::from_str(json).ok())
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (&x, &y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        None
    } else {
        Some(dot / (norm_a.sqrt() * norm_b.sqrt()))
    }
}

impl Tool for MemorySearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_search".into(),
            description: "Search across all memory files for keyword matches. Returns relevant excerpts from matching memories.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Keywords to search for across memory files"
                    }
                },
                "required": ["query"]
            }),
            title: Some("Memory Search".into()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("memory".into()),
            tags: vec!["memory".into(), "search".into()],
            timeout_secs: Some(15),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let query = call
            .input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput {
                message: "Missing or invalid 'query' argument".into(),
            })?;

        let keywords = parse_query_keywords(query)?;
        let matches = keyword_search_results(&self.memory_dir, &keywords);

        Ok(ToolResult::json(
            &call.call_id,
            &call.tool_name,
            json!({
                "query": query,
                "matches": matches,
                "total": matches.len(),
            }),
        ))
    }
}

// ── MemorySimilarTool ───────────────────────────────────────────────

/// Semantic retrieval over the shared workspace Lance journal.
///
/// Falls back to keyword search when embeddings or the workspace journal are
/// unavailable, preserving a useful search experience in lower capability modes.
pub struct MemorySimilarTool {
    memory_dir: PathBuf,
    embedding_provider: Option<Arc<dyn crate::embedding::EmbeddingProvider>>,
    workspace_journal: Option<Arc<LanceJournal>>,
}

impl MemorySimilarTool {
    pub fn new(
        memory_dir: &Path,
        embedding_provider: Option<Arc<dyn crate::embedding::EmbeddingProvider>>,
        workspace_journal: Option<Arc<LanceJournal>>,
    ) -> Self {
        Self {
            memory_dir: memory_dir.to_path_buf(),
            embedding_provider,
            workspace_journal,
        }
    }
}

impl Tool for MemorySimilarTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_similar".into(),
            description: "Find semantically similar memories using workspace embeddings. Falls back to keyword search when vector retrieval is unavailable.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language query describing the memory you want to retrieve"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of memories to return (default: 5)"
                    }
                },
                "required": ["query"]
            }),
            title: Some("Memory Similar".into()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("memory".into()),
            tags: vec!["memory".into(), "semantic".into(), "search".into()],
            timeout_secs: Some(20),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let query = call
            .input
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput {
                message: "Missing or invalid 'query' argument".into(),
            })?;
        let limit = call
            .input
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5) as usize;

        let keywords = parse_query_keywords(query)?;

        let semantic_results = self
            .embedding_provider
            .as_ref()
            .zip(self.workspace_journal.as_ref())
            .and_then(|(provider, journal)| {
                let query_embedding = match provider.embed(query) {
                    Ok(embedding) => embedding,
                    Err(e) => {
                        tracing::warn!(error = %e, "memory_similar embedding failed, falling back");
                        return None;
                    }
                };

                let events = match run_async(journal.vector_search(&query_embedding, limit)) {
                    Ok(events) => events,
                    Err(err) => {
                        tracing::warn!(message = %err, "memory_similar vector search failed, falling back");
                        return None;
                    }
                };

                let keyword_refs: Vec<&str> = keywords.iter().map(String::as_str).collect();
                let mut results = Vec::new();
                for event in events {
                    let Some(content) = event_memory_content(&event) else {
                        continue;
                    };
                    let similarity = event_embedding(&event)
                        .as_deref()
                        .and_then(|embedding| cosine_similarity(&query_embedding, embedding))
                        .unwrap_or(0.0);
                    let title = event
                        .metadata
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| "memory".to_string());

                    results.push(json!({
                        "title": title,
                        "session_id": event.session_id.as_str(),
                        "relevance": similarity,
                        "excerpt": extract_excerpt(content, &keyword_refs, 2),
                        "backend": "vector",
                    }));
                }

                results.sort_by(|a, b| {
                    b.get("relevance")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or(0.0)
                        .partial_cmp(
                            &a.get("relevance")
                                .and_then(serde_json::Value::as_f64)
                                .unwrap_or(0.0),
                        )
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                Some(results)
            });

        let matches =
            semantic_results.unwrap_or_else(|| keyword_search_results(&self.memory_dir, &keywords));
        let backend = matches
            .first()
            .and_then(|m| m.get("backend"))
            .and_then(|v| v.as_str())
            .unwrap_or("keyword");

        Ok(ToolResult::json(
            &call.call_id,
            &call.tool_name,
            json!({
                "query": query,
                "matches": matches,
                "total": matches.len(),
                "backend": backend,
            }),
        ))
    }
}

/// Extract a short excerpt around the first keyword match.
fn extract_excerpt(content: &str, keywords: &[&str], context_lines: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let content_lower = content.to_lowercase();

    for (i, line) in content_lower.lines().enumerate() {
        if keywords.iter().any(|kw| line.contains(kw)) {
            let start = i.saturating_sub(context_lines);
            let end = (i + context_lines + 1).min(lines.len());
            let excerpt: Vec<&str> = lines[start..end].to_vec();
            let result = excerpt.join("\n");
            // Cap excerpt at 500 chars
            if result.len() > 500 {
                return format!("{}...", &result[..500]);
            }
            return result;
        }
    }

    // Fallback: return first few lines
    let preview: String = lines.iter().take(5).copied().collect::<Vec<_>>().join("\n");
    if preview.len() > 300 {
        format!("{}...", &preview[..300])
    } else {
        preview
    }
}

// ── MemoryBrowseTool ────────────────────────────────────────────────

/// List memories grouped by type from YAML frontmatter.
pub struct MemoryBrowseTool {
    memory_dir: PathBuf,
}

impl MemoryBrowseTool {
    pub fn new(memory_dir: &Path) -> Self {
        Self {
            memory_dir: memory_dir.to_path_buf(),
        }
    }
}

impl Tool for MemoryBrowseTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_browse".into(),
            description: "List memories grouped by type/tier. Filter by tier (episodic, procedural, semantic) or type (feedback, finding, session).".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "tier": {
                        "type": "string",
                        "description": "Filter by memory tier (episodic, procedural, semantic). Optional."
                    },
                    "type": {
                        "type": "string",
                        "description": "Filter by memory type (feedback, finding, session, etc.). Optional."
                    }
                }
            }),
            title: Some("Memory Browse".into()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("memory".into()),
            tags: vec!["memory".into(), "browse".into()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let filter_tier = call
            .input
            .get("tier")
            .and_then(|v| v.as_str())
            .map(str::to_lowercase);
        let filter_type = call
            .input
            .get("type")
            .and_then(|v| v.as_str())
            .map(str::to_lowercase);

        let mut memories = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md")
                    && let Ok(content) = fs::read_to_string(&path)
                {
                    let file_name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let frontmatter = parse_simple_frontmatter(&content);
                    let tier = frontmatter
                        .get("tier")
                        .cloned()
                        .unwrap_or_else(|| "unknown".into());
                    let mem_type = frontmatter
                        .get("type")
                        .cloned()
                        .unwrap_or_else(|| "unknown".into());
                    let title = frontmatter
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| file_name.clone());

                    // Apply filters
                    if let Some(ref ft) = filter_tier
                        && tier.to_lowercase() != *ft
                    {
                        continue;
                    }
                    if let Some(ref fty) = filter_type
                        && mem_type.to_lowercase() != *fty
                    {
                        continue;
                    }

                    memories.push(json!({
                        "file": file_name,
                        "title": title,
                        "tier": tier,
                        "type": mem_type,
                    }));
                }
            }
        }

        // Sort by tier then file name for stable output
        memories.sort_by(|a, b| {
            let tier_a = a.get("tier").and_then(|v| v.as_str()).unwrap_or("");
            let tier_b = b.get("tier").and_then(|v| v.as_str()).unwrap_or("");
            tier_a.cmp(tier_b).then_with(|| {
                let fa = a.get("file").and_then(|v| v.as_str()).unwrap_or("");
                let fb = b.get("file").and_then(|v| v.as_str()).unwrap_or("");
                fa.cmp(fb)
            })
        });

        Ok(ToolResult::json(
            &call.call_id,
            &call.tool_name,
            json!({
                "memories": memories,
                "total": memories.len(),
                "filter": {
                    "tier": filter_tier,
                    "type": filter_type,
                },
            }),
        ))
    }
}

/// Parse simple YAML frontmatter from a markdown file.
///
/// Expects `---` delimiters with `key: value` pairs.
fn parse_simple_frontmatter(content: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();

    if !content.starts_with("---") {
        return map;
    }

    let rest = &content[3..];
    if let Some(end) = rest.find("---") {
        let frontmatter = &rest[..end];
        for line in frontmatter.lines() {
            let line = line.trim();
            if let Some((key, value)) = line.split_once(':') {
                let key = key.trim().to_lowercase();
                let value = value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string();
                if !key.is_empty() && !value.is_empty() {
                    map.insert(key, value);
                }
            }
        }
    }

    map
}

// ── MemoryRecentTool ────────────────────────────────────────────────

/// Return the most recently modified memory files with previews.
pub struct MemoryRecentTool {
    memory_dir: PathBuf,
}

impl MemoryRecentTool {
    pub fn new(memory_dir: &Path) -> Self {
        Self {
            memory_dir: memory_dir.to_path_buf(),
        }
    }
}

impl Tool for MemoryRecentTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_recent".into(),
            description: "List the most recently modified memory files with short previews.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of memories to return (default: 5)"
                    }
                }
            }),
            title: Some("Memory Recent".into()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("memory".into()),
            tags: vec!["memory".into(), "recent".into()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let limit = call
            .input
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(5) as usize;

        let mut files: Vec<(PathBuf, std::time::SystemTime)> = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md")
                    && let Ok(meta) = fs::metadata(&path)
                    && let Ok(modified) = meta.modified()
                {
                    files.push((path, modified));
                }
            }
        }

        // Sort by modification time descending (most recent first)
        files.sort_by(|a, b| b.1.cmp(&a.1));

        let results: Vec<serde_json::Value> = files
            .into_iter()
            .take(limit)
            .map(|(path, mtime)| {
                let file_name = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                let preview = fs::read_to_string(&path)
                    .ok()
                    .map(|content| {
                        let lines: Vec<&str> = content.lines().take(6).collect();
                        let preview = lines.join("\n");
                        if preview.len() > 300 {
                            format!("{}...", &preview[..300])
                        } else {
                            preview
                        }
                    })
                    .unwrap_or_default();

                let age_secs = mtime.elapsed().map(|d| d.as_secs()).unwrap_or(0);

                json!({
                    "file": file_name,
                    "age_secs": age_secs,
                    "preview": preview,
                })
            })
            .collect();

        Ok(ToolResult::json(
            &call.call_id,
            &call.tool_name,
            json!({
                "memories": results,
                "total": results.len(),
            }),
        ))
    }
}

// ── MemoryOffloadTool ───────────────────────────────────────────────

/// Save content to a new episodic memory file with YAML frontmatter.
pub struct MemoryOffloadTool {
    memory_dir: PathBuf,
}

impl MemoryOffloadTool {
    pub fn new(memory_dir: &Path) -> Self {
        Self {
            memory_dir: memory_dir.to_path_buf(),
        }
    }
}

impl Tool for MemoryOffloadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_offload".into(),
            description: "Save content to a new memory file with YAML frontmatter. Creates a .md file in .arcan/memory/ and updates the MEMORY.md index.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Memory title (used as filename, e.g. 'auth-fix-finding')"
                    },
                    "content": {
                        "type": "string",
                        "description": "The content to save as memory"
                    },
                    "type": {
                        "type": "string",
                        "description": "Memory type: episodic, procedural, semantic, feedback, finding, session"
                    }
                },
                "required": ["title", "content", "type"]
            }),
            title: Some("Memory Offload".into()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                destructive: true,
                ..Default::default()
            }),
            category: Some("memory".into()),
            tags: vec!["memory".into(), "write".into()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let title = call
            .input
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput {
                message: "Missing or invalid 'title' argument".into(),
            })?;

        let content = call
            .input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput {
                message: "Missing or invalid 'content' argument".into(),
            })?;

        let mem_type = call
            .input
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput {
                message: "Missing or invalid 'type' argument".into(),
            })?;

        // Sanitize title for use as filename
        let file_key: String = title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();

        if file_key.is_empty() || file_key.len() > 64 {
            return Err(ToolError::InvalidInput {
                message: "Title must be 1-64 characters after sanitization".into(),
            });
        }

        fs::create_dir_all(&self.memory_dir).map_err(|e| ToolError::ExecutionFailed {
            tool_name: "memory_offload".into(),
            message: format!("Failed to create memory directory: {e}"),
        })?;

        // Build file content with YAML frontmatter
        let now = chrono::Utc::now().to_rfc3339();
        let file_content = format!(
            "---\ntitle: \"{title}\"\ntype: {mem_type}\ntier: episodic\nimportance: 0.5\ncreated: \"{now}\"\n---\n\n{content}\n"
        );

        let file_path = self.memory_dir.join(format!("{file_key}.md"));
        fs::write(&file_path, &file_content).map_err(|e| ToolError::ExecutionFailed {
            tool_name: "memory_offload".into(),
            message: format!("Failed to write memory file: {e}"),
        })?;

        // Update MEMORY.md index
        update_memory_index(&self.memory_dir, &file_key, title, mem_type);

        Ok(ToolResult::text(
            &call.call_id,
            &call.tool_name,
            &format!("Memory saved: {}", file_path.display()),
        ))
    }
}

/// Append an entry to MEMORY.md index if not already present.
fn update_memory_index(memory_dir: &Path, file_key: &str, title: &str, mem_type: &str) {
    let index_path = memory_dir.join("MEMORY.md");
    let existing = fs::read_to_string(&index_path).unwrap_or_default();

    // Check if entry already exists
    if existing.contains(&format!("({file_key}.md)")) {
        return;
    }

    let entry = format!("- [{title}]({file_key}.md) -- {mem_type}\n");

    let new_content = if existing.is_empty() {
        format!("# Memory Index\n\n{entry}")
    } else {
        format!("{existing}{entry}")
    };

    let _ = fs::write(&index_path, new_content);
}

// ── MemoryForgetTool ────────────────────────────────────────────────

/// Mark a memory as low importance by updating its frontmatter.
pub struct MemoryForgetTool {
    memory_dir: PathBuf,
}

impl MemoryForgetTool {
    pub fn new(memory_dir: &Path) -> Self {
        Self {
            memory_dir: memory_dir.to_path_buf(),
        }
    }
}

impl Tool for MemoryForgetTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "memory_forget".into(),
            description: "Mark a memory as low importance (sets importance: 0.1 in frontmatter). Does not delete the file.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": {
                        "type": "string",
                        "description": "Memory key (filename without .md extension)"
                    }
                },
                "required": ["key"]
            }),
            title: Some("Memory Forget".into()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                destructive: true,
                ..Default::default()
            }),
            category: Some("memory".into()),
            tags: vec!["memory".into(), "forget".into()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let key = call
            .input
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput {
                message: "Missing or invalid 'key' argument".into(),
            })?;

        // Validate key to prevent path traversal
        if key.is_empty()
            || key.len() > 64
            || !key
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            || key.starts_with('.')
            || key.contains("..")
        {
            return Err(ToolError::InvalidInput {
                message: "Invalid memory key".into(),
            });
        }

        let file_path = self.memory_dir.join(format!("{key}.md"));

        if !file_path.exists() {
            return Err(ToolError::ExecutionFailed {
                tool_name: "memory_forget".into(),
                message: format!("Memory file not found: {key}"),
            });
        }

        let content = fs::read_to_string(&file_path).map_err(|e| ToolError::ExecutionFailed {
            tool_name: "memory_forget".into(),
            message: format!("Failed to read memory file: {e}"),
        })?;

        // Update or insert importance: 0.1 in frontmatter
        let new_content = set_frontmatter_importance(&content, 0.1);

        fs::write(&file_path, &new_content).map_err(|e| ToolError::ExecutionFailed {
            tool_name: "memory_forget".into(),
            message: format!("Failed to write memory file: {e}"),
        })?;

        Ok(ToolResult::text(
            &call.call_id,
            &call.tool_name,
            &format!("Memory '{key}' marked as low importance (0.1)"),
        ))
    }
}

/// Set or replace the `importance` field in YAML frontmatter.
fn set_frontmatter_importance(content: &str, importance: f64) -> String {
    if !content.starts_with("---") {
        // No frontmatter — add it
        return format!("---\nimportance: {importance}\n---\n\n{content}");
    }

    let rest = &content[3..];
    if let Some(end) = rest.find("---") {
        let frontmatter = &rest[..end];
        let after = &rest[end + 3..];

        // Check if importance already exists
        let mut found = false;
        let updated_lines: Vec<String> = frontmatter
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("importance:") {
                    found = true;
                    format!("importance: {importance}")
                } else {
                    line.to_string()
                }
            })
            .collect();

        let new_frontmatter = if found {
            updated_lines.join("\n")
        } else {
            format!("{}\nimportance: {importance}", updated_lines.join("\n"))
        };

        format!("---{new_frontmatter}---{after}")
    } else {
        // Malformed frontmatter — return as-is
        content.to_string()
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::tool::{ToolCall, ToolContext};
    use lago_core::event::EventPayload;
    use lago_core::id::{BranchId, EventId, SessionId};
    use tempfile::TempDir;

    struct TestEmbeddingProvider {
        vector: Vec<f32>,
    }

    impl crate::embedding::EmbeddingProvider for TestEmbeddingProvider {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, anyhow::Error> {
            Ok(self.vector.clone())
        }
    }

    fn make_ctx() -> ToolContext {
        ToolContext {
            run_id: "test-run".into(),
            session_id: "test".into(),
            iteration: 0,
        }
    }

    fn make_call(name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall {
            call_id: "test-call".into(),
            tool_name: name.into(),
            input,
            requested_capabilities: vec![],
        }
    }

    fn create_memory_file(dir: &std::path::Path, name: &str, content: &str) {
        fs::write(dir.join(format!("{name}.md")), content).unwrap();
    }

    fn make_memory_event(content: &str, title: &str) -> EventEnvelope {
        EventEnvelope {
            event_id: EventId::new(),
            session_id: SessionId::from_string("workspace"),
            branch_id: BranchId::from_string("main"),
            run_id: None,
            seq: 0,
            timestamp: EventEnvelope::now_micros(),
            parent_id: None,
            payload: EventPayload::Message {
                role: "memory".to_string(),
                content: content.to_string(),
                model: None,
                token_usage: None,
            },
            metadata: {
                let mut map = std::collections::HashMap::new();
                map.insert("title".to_string(), title.to_string());
                map
            },
            schema_version: 1,
        }
    }

    // ── MemorySearchTool tests ──

    #[test]
    fn search_finds_keyword_matches() {
        let dir = TempDir::new().unwrap();
        create_memory_file(
            dir.path(),
            "auth-notes",
            "---\ntitle: Auth Notes\ntype: finding\n---\n\nThe auth middleware was causing issues.",
        );
        create_memory_file(
            dir.path(),
            "deploy-notes",
            "---\ntitle: Deploy Notes\ntype: procedural\n---\n\nDeploy process uses Docker.",
        );

        let tool = MemorySearchTool::new(dir.path());
        let call = make_call("memory_search", json!({"query": "auth middleware"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        assert!(!result.is_error);
        let matches = result.output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0]["file"], "auth-notes");
    }

    #[test]
    fn search_returns_empty_for_no_match() {
        let dir = TempDir::new().unwrap();
        create_memory_file(dir.path(), "notes", "---\ntitle: Notes\n---\n\nSome text.");

        let tool = MemorySearchTool::new(dir.path());
        let call = make_call("memory_search", json!({"query": "nonexistent keyword xyz"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        assert_eq!(result.output["total"], 0);
    }

    #[test]
    fn search_rejects_empty_query() {
        let dir = TempDir::new().unwrap();
        let tool = MemorySearchTool::new(dir.path());
        let call = make_call("memory_search", json!({"query": "   "}));
        let err = tool.execute(&call, &make_ctx()).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    #[test]
    fn similar_uses_vector_search_when_available() {
        let dir = TempDir::new().unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let journal = rt.block_on(async {
            let journal = Arc::new(
                LanceJournal::open(dir.path().join("workspace.lance"))
                    .await
                    .unwrap(),
            );
            journal
                .append_with_embedding(
                    make_memory_event(
                        "Vector memory about auth middleware ordering",
                        "auth-vector",
                    ),
                    vec![1.0; 1536],
                )
                .await
                .unwrap();
            journal
                .append_with_embedding(
                    make_memory_event("Unrelated deployment note", "deploy-vector"),
                    vec![0.0; 1536],
                )
                .await
                .unwrap();
            journal
        });

        let tool = MemorySimilarTool::new(
            dir.path(),
            Some(Arc::new(TestEmbeddingProvider {
                vector: vec![1.0; 1536],
            })),
            Some(journal),
        );

        let call = make_call(
            "memory_similar",
            json!({"query": "auth middleware", "limit": 2}),
        );
        let result = tool.execute(&call, &make_ctx()).unwrap();

        assert_eq!(result.output["backend"], "vector");
        assert_eq!(result.output["matches"][0]["title"], "auth-vector");
        assert!(result.output["matches"][0]["relevance"].as_f64().unwrap() > 0.9);
    }

    #[test]
    fn similar_falls_back_to_keyword_search_without_semantic_backend() {
        let dir = TempDir::new().unwrap();
        create_memory_file(
            dir.path(),
            "auth-notes",
            "---\ntitle: Auth Notes\ntype: finding\n---\n\nThe auth middleware ordering caused the bug.",
        );

        let tool = MemorySimilarTool::new(dir.path(), None, None);
        let call = make_call("memory_similar", json!({"query": "auth middleware"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        assert_eq!(result.output["backend"], "keyword");
        assert_eq!(result.output["matches"][0]["file"], "auth-notes");
    }

    // ── MemoryBrowseTool tests ──

    #[test]
    fn browse_groups_by_type() {
        let dir = TempDir::new().unwrap();
        create_memory_file(
            dir.path(),
            "finding-a",
            "---\ntitle: Finding A\ntype: finding\ntier: episodic\n---\n\nContent A.",
        );
        create_memory_file(
            dir.path(),
            "session-b",
            "---\ntitle: Session B\ntype: session\ntier: procedural\n---\n\nContent B.",
        );

        let tool = MemoryBrowseTool::new(dir.path());

        // No filter — returns all
        let call = make_call("memory_browse", json!({}));
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert_eq!(result.output["total"], 2);

        // Filter by type
        let call = make_call("memory_browse", json!({"type": "finding"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert_eq!(result.output["total"], 1);
        assert_eq!(result.output["memories"][0]["file"], "finding-a");

        // Filter by tier
        let call = make_call("memory_browse", json!({"tier": "procedural"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert_eq!(result.output["total"], 1);
        assert_eq!(result.output["memories"][0]["file"], "session-b");
    }

    // ── MemoryRecentTool tests ──

    #[test]
    fn recent_returns_by_mtime() {
        let dir = TempDir::new().unwrap();
        create_memory_file(dir.path(), "old", "---\ntitle: Old\n---\n\nOld content.");
        // Ensure different mtime
        std::thread::sleep(std::time::Duration::from_millis(50));
        create_memory_file(dir.path(), "new", "---\ntitle: New\n---\n\nNew content.");

        let tool = MemoryRecentTool::new(dir.path());
        let call = make_call("memory_recent", json!({"limit": 1}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        let memories = result.output["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0]["file"], "new");
    }

    #[test]
    fn recent_defaults_to_five() {
        let dir = TempDir::new().unwrap();
        for i in 0..8 {
            create_memory_file(
                dir.path(),
                &format!("mem-{i}"),
                &format!("---\ntitle: Mem {i}\n---\n\nContent {i}."),
            );
        }

        let tool = MemoryRecentTool::new(dir.path());
        let call = make_call("memory_recent", json!({}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        let memories = result.output["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 5);
    }

    // ── MemoryOffloadTool tests ──

    #[test]
    fn offload_creates_file_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryOffloadTool::new(dir.path());

        let call = make_call(
            "memory_offload",
            json!({
                "title": "auth-fix-finding",
                "content": "Root cause: middleware ordering",
                "type": "episodic"
            }),
        );
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert!(!result.is_error);

        // Verify file was created
        let file_path = dir.path().join("auth-fix-finding.md");
        assert!(file_path.exists());

        let content = fs::read_to_string(&file_path).unwrap();
        assert!(content.contains("title: \"auth-fix-finding\""));
        assert!(content.contains("type: episodic"));
        assert!(content.contains("tier: episodic"));
        assert!(content.contains("importance: 0.5"));
        assert!(content.contains("Root cause: middleware ordering"));
    }

    #[test]
    fn offload_updates_memory_index() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryOffloadTool::new(dir.path());

        let call = make_call(
            "memory_offload",
            json!({
                "title": "my-finding",
                "content": "Some content",
                "type": "finding"
            }),
        );
        tool.execute(&call, &make_ctx()).unwrap();

        let index = fs::read_to_string(dir.path().join("MEMORY.md")).unwrap();
        assert!(index.contains("my-finding"));
        assert!(index.contains("finding"));
    }

    #[test]
    fn offload_sanitizes_title() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryOffloadTool::new(dir.path());

        let call = make_call(
            "memory_offload",
            json!({
                "title": "my finding/with spaces",
                "content": "content",
                "type": "episodic"
            }),
        );
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert!(!result.is_error);

        // Spaces and slashes get replaced with hyphens
        let file_path = dir.path().join("my-finding-with-spaces.md");
        assert!(file_path.exists());
    }

    // ── MemoryForgetTool tests ──

    #[test]
    fn forget_updates_frontmatter_importance() {
        let dir = TempDir::new().unwrap();
        create_memory_file(
            dir.path(),
            "old-summary",
            "---\ntitle: Old Summary\nimportance: 0.5\n---\n\nContent.",
        );

        let tool = MemoryForgetTool::new(dir.path());
        let call = make_call("memory_forget", json!({"key": "old-summary"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();
        assert!(!result.is_error);

        let content = fs::read_to_string(dir.path().join("old-summary.md")).unwrap();
        assert!(content.contains("importance: 0.1"));
        assert!(!content.contains("importance: 0.5"));
    }

    #[test]
    fn forget_adds_importance_if_missing() {
        let dir = TempDir::new().unwrap();
        create_memory_file(
            dir.path(),
            "no-importance",
            "---\ntitle: No Importance\n---\n\nContent.",
        );

        let tool = MemoryForgetTool::new(dir.path());
        let call = make_call("memory_forget", json!({"key": "no-importance"}));
        tool.execute(&call, &make_ctx()).unwrap();

        let content = fs::read_to_string(dir.path().join("no-importance.md")).unwrap();
        assert!(content.contains("importance: 0.1"));
    }

    #[test]
    fn forget_nonexistent_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryForgetTool::new(dir.path());
        let call = make_call("memory_forget", json!({"key": "nonexistent"}));
        let err = tool.execute(&call, &make_ctx()).unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed { .. }));
    }

    #[test]
    fn forget_rejects_invalid_key() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryForgetTool::new(dir.path());

        let call = make_call("memory_forget", json!({"key": "../escape"}));
        let err = tool.execute(&call, &make_ctx()).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
    }

    // ── Frontmatter helper tests ──

    #[test]
    fn parse_simple_frontmatter_works() {
        let content = "---\ntitle: Test\ntype: finding\ntier: episodic\n---\n\nBody text.";
        let fm = parse_simple_frontmatter(content);
        assert_eq!(fm.get("title").unwrap(), "Test");
        assert_eq!(fm.get("type").unwrap(), "finding");
        assert_eq!(fm.get("tier").unwrap(), "episodic");
    }

    #[test]
    fn parse_frontmatter_no_frontmatter() {
        let content = "Just some text.";
        let fm = parse_simple_frontmatter(content);
        assert!(fm.is_empty());
    }

    #[test]
    fn set_importance_replaces_existing() {
        let content = "---\ntitle: Test\nimportance: 0.8\n---\n\nBody.";
        let result = set_frontmatter_importance(content, 0.1);
        assert!(result.contains("importance: 0.1"));
        assert!(!result.contains("importance: 0.8"));
    }

    #[test]
    fn set_importance_adds_when_missing() {
        let content = "---\ntitle: Test\n---\n\nBody.";
        let result = set_frontmatter_importance(content, 0.1);
        assert!(result.contains("importance: 0.1"));
    }

    #[test]
    fn set_importance_handles_no_frontmatter() {
        let content = "Just plain text.";
        let result = set_frontmatter_importance(content, 0.1);
        assert!(result.contains("---"));
        assert!(result.contains("importance: 0.1"));
        assert!(result.contains("Just plain text."));
    }

    // ── extract_excerpt tests ──

    #[test]
    fn extract_excerpt_finds_keyword_context() {
        let content = "Line 1\nLine 2\nThe auth middleware failed\nLine 4\nLine 5";
        let excerpt = extract_excerpt(content, &["auth"], 1);
        assert!(excerpt.contains("auth middleware"));
        assert!(excerpt.contains("Line 2")); // context line before
    }

    // ── Tool definition tests ──

    #[test]
    fn all_tools_have_valid_definitions() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        let tools: Vec<Box<dyn Tool>> = vec![
            Box::new(MemorySearchTool::new(&path)),
            Box::new(MemorySimilarTool::new(&path, None, None)),
            Box::new(MemoryBrowseTool::new(&path)),
            Box::new(MemoryRecentTool::new(&path)),
            Box::new(MemoryOffloadTool::new(&path)),
            Box::new(MemoryForgetTool::new(&path)),
        ];

        let expected_names = [
            "memory_search",
            "memory_similar",
            "memory_browse",
            "memory_recent",
            "memory_offload",
            "memory_forget",
        ];

        for (tool, expected_name) in tools.iter().zip(expected_names.iter()) {
            let def = tool.definition();
            assert_eq!(def.name, *expected_name);
            assert!(!def.description.is_empty());
            assert_eq!(def.category.as_deref(), Some("memory"));
        }
    }
}
