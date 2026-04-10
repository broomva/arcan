//! Bounded memory graph retrieval over `lago-knowledge`.
//!
//! The graph is a derived view over markdown memory artifacts. This adapter
//! shapes Lago traversal primitives into compact, provenance-preserving payloads
//! that Arcan can expose as an agent tool.

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Path;

use chrono::{DateTime, NaiveDate, TimeZone, Utc};
use lago_knowledge::{KnowledgeError, KnowledgeIndex};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::build_index_from_dir;

pub const DEFAULT_GRAPH_DEPTH: usize = 2;
pub const DEFAULT_MAX_NODES: usize = 12;
pub const DEFAULT_MAX_EDGES: usize = 16;
pub const MAX_GRAPH_DEPTH: usize = 4;
pub const MAX_GRAPH_NODES: usize = 50;
pub const MAX_GRAPH_EDGES: usize = 100;

/// Query parameters for bounded memory graph retrieval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryGraphQuery {
    pub start: String,
    pub query: Option<String>,
    pub depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub edge_types: Vec<String>,
}

impl MemoryGraphQuery {
    pub fn new(start: impl Into<String>) -> Self {
        Self {
            start: start.into(),
            query: None,
            depth: DEFAULT_GRAPH_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
            max_edges: DEFAULT_MAX_EDGES,
            edge_types: Vec::new(),
        }
    }

    pub fn bounded(mut self) -> Self {
        self.query = self
            .query
            .map(|query| query.trim().to_string())
            .filter(|query| !query.is_empty());
        self.depth = self.depth.min(MAX_GRAPH_DEPTH);
        self.max_nodes = self.max_nodes.clamp(1, MAX_GRAPH_NODES);
        self.max_edges = self.max_edges.min(MAX_GRAPH_EDGES);
        self.edge_types = self
            .edge_types
            .into_iter()
            .map(|edge| edge.trim().to_lowercase())
            .filter(|edge| !edge.is_empty())
            .collect();
        self
    }

    fn includes_references(&self) -> bool {
        self.edge_types.is_empty() || self.edge_types.iter().any(|edge| edge == "references")
    }
}

/// Optional rank hints from semantic retrieval backends.
///
/// `arcan-lago` deliberately accepts score hints instead of depending on a
/// concrete vector store. This keeps Lance and embedding providers owned by
/// Arcan while preserving a deterministic graph-only fallback.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoryGraphRankingHints {
    pub semantic_scores: HashMap<String, f32>,
    pub fallback_path: Option<String>,
}

impl MemoryGraphRankingHints {
    pub fn new(semantic_scores: HashMap<String, f32>) -> Self {
        Self {
            semantic_scores: semantic_scores
                .into_iter()
                .map(|(key, score)| (ranking_key(&key), clamp_unit(score)))
                .collect(),
            fallback_path: None,
        }
    }

    pub fn with_fallback_path(mut self, fallback_path: impl Into<String>) -> Self {
        let fallback_path = fallback_path.into();
        self.fallback_path = if fallback_path.trim().is_empty() {
            None
        } else {
            Some(fallback_path)
        };
        self
    }

    fn has_semantic_scores(&self) -> bool {
        !self.semantic_scores.is_empty()
    }

    fn semantic_score_for_note(&self, note: &lago_knowledge::Note) -> f32 {
        if self.semantic_scores.is_empty() {
            return 0.0;
        }

        let mut keys = vec![note.path.clone(), note.name.clone()];
        if let Some(title) = frontmatter_string(note, &["title", "core_claim", "description"]) {
            keys.push(title);
        }

        keys.iter()
            .filter_map(|key| {
                self.semantic_scores
                    .get(&ranking_key(key))
                    .copied()
                    .or_else(|| self.semantic_scores.get(key.as_str()).copied())
            })
            .map(clamp_unit)
            .fold(0.0, f32::max)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryGraphNode {
    pub node_id: String,
    pub node_type: String,
    pub title: String,
    pub summary: String,
    pub source_ref: String,
    pub depth: usize,
    pub outgoing_links: Vec<String>,
    pub score: f32,
    pub rank_signals: MemoryGraphRankSignals,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryGraphRankSignals {
    pub depth: f32,
    pub query: f32,
    pub semantic: f32,
    pub importance: f32,
    pub recency: f32,
    pub edge_weight: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryGraphEdge {
    pub source: String,
    pub target: String,
    pub edge_type: String,
    pub label: String,
    pub source_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryGraphResponse {
    pub found: bool,
    pub start: String,
    pub root: Option<String>,
    pub nodes: Vec<MemoryGraphNode>,
    pub edges: Vec<MemoryGraphEdge>,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub truncated: bool,
    pub depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub edge_filter: Vec<String>,
    pub query: Option<String>,
    pub ranking_backend: String,
    pub metrics: MemoryGraphMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryGraphMetrics {
    pub operation: String,
    pub returned_node_count: usize,
    pub returned_edge_count: usize,
    pub depth_reached: usize,
    pub ranking_backend: String,
    pub fallback_path: Option<String>,
    pub provenance_preserved: bool,
}

#[derive(Debug, Error)]
pub enum MemoryGraphError {
    #[error("failed to build knowledge index: {0}")]
    Index(#[from] KnowledgeError),
    #[error("memory graph start node not found: {0}")]
    StartNodeNotFound(String),
}

/// Build a graph response from markdown files in `memory_dir`.
pub fn memory_graph_from_dir(
    memory_dir: &Path,
    query: MemoryGraphQuery,
) -> Result<MemoryGraphResponse, MemoryGraphError> {
    memory_graph_from_dir_with_ranking(memory_dir, query, MemoryGraphRankingHints::default())
}

/// Build a graph response from markdown files with optional rank hints.
pub fn memory_graph_from_dir_with_ranking(
    memory_dir: &Path,
    query: MemoryGraphQuery,
    ranking_hints: MemoryGraphRankingHints,
) -> Result<MemoryGraphResponse, MemoryGraphError> {
    let (index, _store) = build_index_from_dir(memory_dir)?;
    memory_graph_from_index_with_ranking(&index, query, ranking_hints)
}

/// Build a graph response from an already-built knowledge index.
pub fn memory_graph_from_index(
    index: &KnowledgeIndex,
    query: MemoryGraphQuery,
) -> Result<MemoryGraphResponse, MemoryGraphError> {
    memory_graph_from_index_with_ranking(index, query, MemoryGraphRankingHints::default())
}

/// Build a graph response with optional query-conditioned and semantic ranking.
pub fn memory_graph_from_index_with_ranking(
    index: &KnowledgeIndex,
    query: MemoryGraphQuery,
    ranking_hints: MemoryGraphRankingHints,
) -> Result<MemoryGraphResponse, MemoryGraphError> {
    let query = query.bounded();
    let Some(start_note) = index.resolve_note_ref(&query.start) else {
        return Err(MemoryGraphError::StartNodeNotFound(query.start));
    };

    let effective_depth = if query.includes_references() {
        query.depth
    } else {
        0
    };

    let ranked_mode = query.query.is_some() || ranking_hints.has_semantic_scores();
    let traversal_limit = if ranked_mode {
        query
            .max_nodes
            .saturating_mul(4)
            .max(query.max_nodes.saturating_add(1))
            .min(MAX_GRAPH_NODES.saturating_add(1))
    } else {
        query.max_nodes.saturating_add(1)
    };

    let mut traversal = index.traverse(&start_note.path, effective_depth, traversal_limit);
    let candidates_overflowed = traversal.len() > query.max_nodes;

    if ranked_mode {
        traversal = rank_traversal(
            index,
            &start_note.path,
            traversal,
            effective_depth,
            &query,
            &ranking_hints,
        )
        .into_iter()
        .map(|ranked| ranked.node)
        .collect();
    }

    traversal.truncate(query.max_nodes);

    let returned_paths: HashSet<&str> = traversal.iter().map(|node| node.path.as_str()).collect();

    let graph_edges = if query.includes_references() {
        graph_edges(index, &traversal, &returned_paths, query.max_edges)
    } else {
        GraphEdges::default()
    };
    let edges_overflowed = graph_edges.overflowed;
    let mut outgoing_links_by_source = graph_edges.outgoing_links_by_source;
    let edge_weights = graph_edge_weights(index, &traversal, &returned_paths);
    let search_terms = query_terms(query.query.as_deref());

    let nodes = traversal
        .iter()
        .filter_map(|node| {
            let note = index.get_note(&node.path)?;
            let rank_signals = rank_signals(
                note,
                node.depth,
                effective_depth,
                &search_terms,
                &ranking_hints,
                edge_weights.get(note.path.as_str()).copied().unwrap_or(0.0),
            );
            let score = composite_score(&rank_signals, ranked_mode);

            Some(MemoryGraphNode {
                node_id: note.path.clone(),
                node_type: note_type(note),
                title: note_title(note),
                summary: note_summary(note),
                source_ref: note.path.clone(),
                depth: node.depth,
                outgoing_links: outgoing_links_by_source
                    .remove(&note.path)
                    .unwrap_or_default(),
                score,
                rank_signals,
            })
        })
        .collect::<Vec<_>>();

    let truncated = candidates_overflowed || edges_overflowed;
    let ranking_backend = ranking_backend(ranked_mode, ranking_hints.has_semantic_scores());
    let metrics = graph_metrics(
        &nodes,
        &graph_edges.edges,
        &ranking_backend,
        ranking_hints.fallback_path.clone(),
    );

    Ok(MemoryGraphResponse {
        found: true,
        start: query.start,
        root: Some(start_note.path.clone()),
        total_nodes: nodes.len(),
        total_edges: graph_edges.edges.len(),
        nodes,
        edges: graph_edges.edges,
        truncated,
        depth: effective_depth,
        max_nodes: query.max_nodes,
        max_edges: query.max_edges,
        edge_filter: if query.edge_types.is_empty() {
            vec!["references".to_string()]
        } else {
            query.edge_types
        },
        query: query.query,
        ranking_backend,
        metrics,
    })
}

fn graph_metrics(
    nodes: &[MemoryGraphNode],
    edges: &[MemoryGraphEdge],
    ranking_backend: &str,
    fallback_path: Option<String>,
) -> MemoryGraphMetrics {
    MemoryGraphMetrics {
        operation: "memory_graph".to_string(),
        returned_node_count: nodes.len(),
        returned_edge_count: edges.len(),
        depth_reached: nodes.iter().map(|node| node.depth).max().unwrap_or(0),
        ranking_backend: ranking_backend.to_string(),
        fallback_path,
        provenance_preserved: graph_provenance_preserved(nodes, edges),
    }
}

fn graph_provenance_preserved(nodes: &[MemoryGraphNode], edges: &[MemoryGraphEdge]) -> bool {
    nodes.iter().all(|node| {
        !node.node_id.trim().is_empty()
            && !node.source_ref.trim().is_empty()
            && node.node_id == node.source_ref
    }) && edges.iter().all(|edge| {
        !edge.source.trim().is_empty()
            && !edge.target.trim().is_empty()
            && !edge.source_ref.trim().is_empty()
            && edge.source == edge.source_ref
    })
}

#[derive(Debug)]
struct RankedTraversal {
    node: lago_knowledge::TraversalResult,
    score: f32,
    original_index: usize,
}

fn rank_traversal(
    index: &KnowledgeIndex,
    root_path: &str,
    traversal: Vec<lago_knowledge::TraversalResult>,
    effective_depth: usize,
    query: &MemoryGraphQuery,
    ranking_hints: &MemoryGraphRankingHints,
) -> Vec<RankedTraversal> {
    let search_terms = query_terms(query.query.as_deref());
    let candidate_paths: HashSet<&str> = traversal.iter().map(|node| node.path.as_str()).collect();
    let edge_weights = graph_edge_weights(index, &traversal, &candidate_paths);

    let mut ranked = traversal
        .into_iter()
        .enumerate()
        .map(|(original_index, node)| {
            let score = index
                .get_note(&node.path)
                .map(|note| {
                    let signals = rank_signals(
                        note,
                        node.depth,
                        effective_depth,
                        &search_terms,
                        ranking_hints,
                        edge_weights.get(note.path.as_str()).copied().unwrap_or(0.0),
                    );
                    composite_score(&signals, true)
                })
                .unwrap_or(0.0);

            RankedTraversal {
                node,
                score,
                original_index,
            }
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|a, b| {
        let a_is_root = a.node.path == root_path;
        let b_is_root = b.node.path == root_path;
        match (a_is_root, b_is_root) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => b
                .score
                .partial_cmp(&a.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.node.depth.cmp(&b.node.depth))
                .then_with(|| a.original_index.cmp(&b.original_index)),
        }
    });

    ranked
}

#[derive(Debug, Default)]
struct GraphEdges {
    edges: Vec<MemoryGraphEdge>,
    outgoing_links_by_source: HashMap<String, Vec<String>>,
    overflowed: bool,
}

fn graph_edges(
    index: &KnowledgeIndex,
    traversal: &[lago_knowledge::TraversalResult],
    returned_paths: &HashSet<&str>,
    max_edges: usize,
) -> GraphEdges {
    let mut edges = Vec::new();
    let mut outgoing_links_by_source: HashMap<String, Vec<String>> = HashMap::new();
    let mut seen = HashSet::new();
    let mut overflowed = false;

    for source in traversal {
        let Some(note) = index.get_note(&source.path) else {
            continue;
        };

        for link in &note.links {
            let Some(target) = index.resolve_note_ref(link) else {
                continue;
            };
            if !returned_paths.contains(target.path.as_str()) {
                continue;
            }
            let key = (note.path.clone(), target.path.clone(), link.clone());
            if !seen.insert(key) {
                continue;
            }

            if edges.len() >= max_edges {
                overflowed = true;
                continue;
            }

            edges.push(MemoryGraphEdge {
                source: note.path.clone(),
                target: target.path.clone(),
                edge_type: "references".to_string(),
                label: link.clone(),
                source_ref: note.path.clone(),
            });
            outgoing_links_by_source
                .entry(note.path.clone())
                .or_default()
                .push(link.clone());
        }
    }

    GraphEdges {
        edges,
        outgoing_links_by_source,
        overflowed,
    }
}

fn graph_edge_weights(
    index: &KnowledgeIndex,
    traversal: &[lago_knowledge::TraversalResult],
    candidate_paths: &HashSet<&str>,
) -> HashMap<String, f32> {
    let mut weights: HashMap<String, usize> = HashMap::new();

    for source in traversal {
        let Some(note) = index.get_note(&source.path) else {
            continue;
        };

        for link in &note.links {
            let Some(target) = index.resolve_note_ref(link) else {
                continue;
            };
            if !candidate_paths.contains(target.path.as_str()) {
                continue;
            }

            *weights.entry(note.path.clone()).or_default() += 1;
            *weights.entry(target.path.clone()).or_default() += 1;
        }
    }

    weights
        .into_iter()
        .map(|(path, weight)| (path, (weight as f32 / 4.0).min(1.0)))
        .collect()
}

fn rank_signals(
    note: &lago_knowledge::Note,
    depth: usize,
    max_depth: usize,
    search_terms: &[String],
    ranking_hints: &MemoryGraphRankingHints,
    edge_weight: f32,
) -> MemoryGraphRankSignals {
    MemoryGraphRankSignals {
        depth: depth_score(depth, max_depth),
        query: lexical_query_score(note, search_terms),
        semantic: ranking_hints.semantic_score_for_note(note),
        importance: frontmatter_unit(note, &["importance", "knowledge_relevance", "priority"])
            .unwrap_or(0.0),
        recency: frontmatter_recency(note),
        edge_weight: clamp_unit(edge_weight),
    }
}

fn composite_score(signals: &MemoryGraphRankSignals, ranked_mode: bool) -> f32 {
    let score = if ranked_mode {
        (signals.depth * 0.20)
            + (signals.query * 0.35)
            + (signals.semantic * 0.25)
            + (signals.importance * 0.10)
            + (signals.recency * 0.05)
            + (signals.edge_weight * 0.05)
    } else {
        (signals.depth * 0.60)
            + (signals.importance * 0.15)
            + (signals.recency * 0.10)
            + (signals.edge_weight * 0.15)
    };

    clamp_unit(score)
}

fn depth_score(depth: usize, max_depth: usize) -> f32 {
    let denominator = max_depth.saturating_add(1).max(1) as f32;
    let remaining = max_depth.saturating_add(1).saturating_sub(depth) as f32;
    clamp_unit(remaining / denominator)
}

fn lexical_query_score(note: &lago_knowledge::Note, search_terms: &[String]) -> f32 {
    if search_terms.is_empty() {
        return 0.0;
    }

    let title = note_title(note).to_lowercase();
    let summary = note_summary(note).to_lowercase();
    let name = note.name.to_lowercase();
    let body = note.body.to_lowercase();
    let tags = frontmatter_tags(note);

    let mut score = 0.0f32;
    for term in search_terms {
        let mut term_score = 0.0f32;
        if title.contains(term) || name.contains(term) {
            term_score += 0.45;
        }
        if summary.contains(term) {
            term_score += 0.25;
        }
        if body.contains(term) {
            term_score += 0.20;
        }
        if tags.iter().any(|tag| tag == term) {
            term_score += 0.10;
        }
        score += term_score.min(1.0);
    }

    clamp_unit(score / search_terms.len() as f32)
}

fn frontmatter_tags(note: &lago_knowledge::Note) -> Vec<String> {
    note.frontmatter
        .get("tags")
        .and_then(|value| value.as_sequence())
        .map(|tags| {
            tags.iter()
                .filter_map(|tag| tag.as_str())
                .map(|tag| tag.trim().to_lowercase())
                .filter(|tag| !tag.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn frontmatter_unit(note: &lago_knowledge::Note, keys: &[&str]) -> Option<f32> {
    keys.iter().find_map(|key| {
        note.frontmatter
            .get(*key)
            .and_then(yaml_number)
            .map(|value| if value > 1.0 { value / 100.0 } else { value })
            .map(clamp_unit)
    })
}

fn frontmatter_recency(note: &lago_knowledge::Note) -> f32 {
    if let Some(score) = frontmatter_unit(note, &["recency"]) {
        return score;
    }

    [
        "updated_at",
        "updated",
        "created_at",
        "created",
        "timestamp",
    ]
    .iter()
    .find_map(|key| note.frontmatter.get(*key).and_then(temporal_score))
    .unwrap_or(0.0)
}

fn temporal_score(value: &serde_yaml::Value) -> Option<f32> {
    let observed_at = if let Some(number) = yaml_number(value) {
        timestamp_to_datetime(number)?
    } else {
        let value = value.as_str()?.trim();
        DateTime::parse_from_rfc3339(value)
            .map(|dt| dt.with_timezone(&Utc))
            .ok()
            .or_else(|| {
                NaiveDate::parse_from_str(value, "%Y-%m-%d")
                    .ok()
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|dt| Utc.from_utc_datetime(&dt))
            })?
    };

    let now = Utc::now();
    let age_days = now.signed_duration_since(observed_at).num_days().max(0) as f32;
    Some(clamp_unit(1.0 / (1.0 + age_days / 30.0)))
}

fn timestamp_to_datetime(timestamp: f32) -> Option<DateTime<Utc>> {
    let timestamp = timestamp as f64;
    let seconds = if timestamp > 1_000_000_000_000_000.0 {
        timestamp / 1_000_000.0
    } else if timestamp > 1_000_000_000_000.0 {
        timestamp / 1_000.0
    } else {
        timestamp
    };

    let whole_seconds = seconds.trunc() as i64;
    let nanos = ((seconds.fract() * 1_000_000_000.0).round() as u32).min(999_999_999);
    Utc.timestamp_opt(whole_seconds, nanos).single()
}

fn yaml_number(value: &serde_yaml::Value) -> Option<f32> {
    value
        .as_f64()
        .map(|value| value as f32)
        .or_else(|| value.as_i64().map(|value| value as f32))
        .or_else(|| value.as_u64().map(|value| value as f32))
        .or_else(|| value.as_str()?.trim().parse::<f32>().ok())
}

fn query_terms(query: Option<&str>) -> Vec<String> {
    query
        .unwrap_or_default()
        .to_lowercase()
        .split_whitespace()
        .map(|term| {
            term.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
                .to_string()
        })
        .filter(|term| !term.is_empty())
        .collect()
}

fn ranking_backend(ranked_mode: bool, has_semantic_scores: bool) -> String {
    match (ranked_mode, has_semantic_scores) {
        (true, true) => "hybrid_vector_graph".to_string(),
        (true, false) => "hybrid_lexical_graph".to_string(),
        (false, _) => "graph_bfs".to_string(),
    }
}

fn ranking_key(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("[[")
        .trim_end_matches("]]")
        .split('#')
        .next()
        .unwrap_or(value)
        .trim()
        .trim_start_matches('/')
        .trim_end_matches(".md")
        .to_lowercase()
}

fn clamp_unit(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn note_title(note: &lago_knowledge::Note) -> String {
    frontmatter_string(note, &["title", "core_claim", "description"])
        .unwrap_or_else(|| note.name.clone())
}

fn note_type(note: &lago_knowledge::Note) -> String {
    frontmatter_string(note, &["node_type", "type", "tier"])
        .map(|value| value.trim().to_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "memory"
                    | "decision"
                    | "evidence"
                    | "outcome"
                    | "pattern"
                    | "artifact"
                    | "session_summary"
            )
        })
        .unwrap_or_else(|| "memory".to_string())
}

fn note_summary(note: &lago_knowledge::Note) -> String {
    frontmatter_string(note, &["summary", "core_claim", "description"])
        .unwrap_or_else(|| body_preview(&note.body))
}

fn frontmatter_string(note: &lago_knowledge::Note, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        note.frontmatter
            .get(*key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn body_preview(body: &str) -> String {
    let summary = body
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .take(4)
        .collect::<Vec<_>>()
        .join(" ");

    truncate_chars(&summary, 280)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let mut truncated = value.chars().take(max_chars).collect::<String>();
    truncated.push_str("...");
    truncated
}

#[cfg(test)]
mod tests {
    use super::*;
    use lago_core::ManifestEntry;
    use lago_store::BlobStore;
    use tempfile::TempDir;

    fn build_index(files: &[(&str, &str)]) -> (TempDir, KnowledgeIndex) {
        let tmp = TempDir::new().unwrap();
        let store = BlobStore::open(tmp.path()).unwrap();
        let entries = files
            .iter()
            .map(|(path, content)| {
                let hash = store.put(content.as_bytes()).unwrap();
                ManifestEntry {
                    path: (*path).to_string(),
                    blob_hash: hash,
                    size_bytes: content.len() as u64,
                    content_type: Some("text/markdown".to_string()),
                    updated_at: 0,
                }
            })
            .collect::<Vec<_>>();
        (tmp, KnowledgeIndex::build(&entries, &store).unwrap())
    }

    #[test]
    fn graph_returns_chain_with_edges_and_provenance() {
        let (_tmp, index) = build_index(&[
            (
                "/decision.md",
                "---\ntitle: Sandbox decision\ntype: decision\nsummary: Choose bounded sandbox routing.\n---\nSee [[Evidence]].",
            ),
            (
                "/evidence.md",
                "---\ntitle: Evidence\ntype: evidence\n---\nSee [[Outcome]].",
            ),
            (
                "/outcome.md",
                "---\ntitle: Outcome\ntype: outcome\n---\nPolicy stayed replay-safe.",
            ),
        ]);

        let graph = memory_graph_from_index(
            &index,
            MemoryGraphQuery {
                start: "Decision".into(),
                query: None,
                depth: 2,
                max_nodes: 12,
                max_edges: 16,
                edge_types: Vec::new(),
            },
        )
        .unwrap();

        assert!(graph.found);
        assert_eq!(graph.root.as_deref(), Some("/decision.md"));
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.nodes[0].node_type, "decision");
        assert_eq!(graph.nodes[0].source_ref, "/decision.md");
        assert_eq!(graph.nodes[0].outgoing_links, vec!["Evidence"]);
        assert_eq!(graph.metrics.operation, "memory_graph");
        assert_eq!(graph.metrics.returned_node_count, 3);
        assert_eq!(graph.metrics.returned_edge_count, 2);
        assert_eq!(graph.metrics.depth_reached, 2);
        assert_eq!(graph.metrics.ranking_backend, "graph_bfs");
        assert!(graph.metrics.fallback_path.is_none());
        assert!(graph.metrics.provenance_preserved);
    }

    #[test]
    fn graph_handles_cycles_without_repeating_nodes() {
        let (_tmp, index) = build_index(&[
            ("/a.md", "---\ntitle: A\n---\nSee [[B]]."),
            ("/b.md", "---\ntitle: B\n---\nSee [[A]]."),
        ]);

        let graph = memory_graph_from_index(&index, MemoryGraphQuery::new("A")).unwrap();

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 2);
        assert!(graph.metrics.provenance_preserved);
    }

    #[test]
    fn graph_respects_node_and_edge_bounds() {
        let (_tmp, index) = build_index(&[
            ("/a.md", "# A\n\nSee [[B]] and [[C]] and [[D]]."),
            ("/b.md", "# B"),
            ("/c.md", "# C"),
            ("/d.md", "# D"),
        ]);

        let graph = memory_graph_from_index(
            &index,
            MemoryGraphQuery {
                start: "/a.md".into(),
                query: None,
                depth: 1,
                max_nodes: 2,
                max_edges: 1,
                edge_types: Vec::new(),
            },
        )
        .unwrap();

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert!(graph.truncated);
        assert_eq!(graph.nodes[0].outgoing_links, vec!["B"]);
    }

    #[test]
    fn graph_does_not_report_truncated_when_result_exactly_matches_limits() {
        let (_tmp, index) =
            build_index(&[("/a.md", "# A\n\nSee [[B]]."), ("/b.md", "# B\n\nEnd.")]);

        let graph = memory_graph_from_index(
            &index,
            MemoryGraphQuery {
                start: "A".into(),
                query: None,
                depth: 1,
                max_nodes: 2,
                max_edges: 1,
                edge_types: Vec::new(),
            },
        )
        .unwrap();

        assert_eq!(graph.nodes.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert!(!graph.truncated);
    }

    #[test]
    fn graph_reports_missing_start_node() {
        let (_tmp, index) = build_index(&[("/a.md", "# A")]);
        let err = memory_graph_from_index(&index, MemoryGraphQuery::new("missing")).unwrap_err();
        assert!(matches!(err, MemoryGraphError::StartNodeNotFound(_)));
    }

    #[test]
    fn graph_edge_filter_excluding_references_returns_root_only() {
        let (_tmp, index) = build_index(&[("/a.md", "# A\n\nSee [[B]]."), ("/b.md", "# B")]);

        let graph = memory_graph_from_index(
            &index,
            MemoryGraphQuery {
                start: "A".into(),
                query: None,
                depth: 2,
                max_nodes: 12,
                max_edges: 16,
                edge_types: vec!["supports".into()],
            },
        )
        .unwrap();

        assert_eq!(graph.nodes.len(), 1);
        assert!(graph.edges.is_empty());
        assert_eq!(graph.edge_filter, vec!["supports"]);
    }

    #[test]
    fn query_conditioned_ranking_promotes_relevant_nodes_within_bounds() {
        let (_tmp, index) = build_index(&[
            (
                "/root.md",
                "---\ntitle: Root\n---\nSee [[Noise]] and [[Calibration]].",
            ),
            (
                "/noise.md",
                "---\ntitle: Noise\n---\nDeployment notes unrelated to evaluation.",
            ),
            (
                "/calibration.md",
                "---\ntitle: Calibration\nimportance: 0.8\n---\nRecall threshold tuning improves knowledge calibration.",
            ),
        ]);

        let plain = memory_graph_from_index(
            &index,
            MemoryGraphQuery {
                start: "Root".into(),
                query: None,
                depth: 1,
                max_nodes: 2,
                max_edges: 16,
                edge_types: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(plain.nodes[1].source_ref, "/noise.md");
        assert_eq!(plain.ranking_backend, "graph_bfs");

        let ranked = memory_graph_from_index(
            &index,
            MemoryGraphQuery {
                start: "Root".into(),
                query: Some("knowledge calibration recall".into()),
                depth: 1,
                max_nodes: 2,
                max_edges: 16,
                edge_types: Vec::new(),
            },
        )
        .unwrap();

        assert_eq!(ranked.nodes.len(), 2);
        assert_eq!(ranked.nodes[0].source_ref, "/root.md");
        assert_eq!(ranked.nodes[1].source_ref, "/calibration.md");
        assert!(ranked.nodes[1].rank_signals.query > plain.nodes[1].rank_signals.query);
        assert!(ranked.truncated);
        assert_eq!(
            ranked.query.as_deref(),
            Some("knowledge calibration recall")
        );
        assert_eq!(ranked.ranking_backend, "hybrid_lexical_graph");
        assert_eq!(ranked.metrics.ranking_backend, "hybrid_lexical_graph");
        assert_eq!(ranked.metrics.returned_node_count, 2);
        assert_eq!(ranked.metrics.depth_reached, 1);
        assert!(ranked.metrics.provenance_preserved);
    }

    #[test]
    fn semantic_hints_can_promote_vector_matched_nodes() {
        let (_tmp, index) = build_index(&[
            (
                "/root.md",
                "---\ntitle: Root\n---\nSee [[Noise]] and [[semantic-target]].",
            ),
            (
                "/noise.md",
                "---\ntitle: Noise\n---\nPlain first BFS result.",
            ),
            (
                "/semantic-target.md",
                "---\ntitle: Semantic Target\n---\nVector-only match.",
            ),
        ]);

        let mut scores = HashMap::new();
        scores.insert("Semantic Target".to_string(), 0.98);

        let graph = memory_graph_from_index_with_ranking(
            &index,
            MemoryGraphQuery {
                start: "Root".into(),
                query: Some("opaque query".into()),
                depth: 1,
                max_nodes: 2,
                max_edges: 16,
                edge_types: Vec::new(),
            },
            MemoryGraphRankingHints::new(scores),
        )
        .unwrap();

        assert_eq!(graph.nodes[1].source_ref, "/semantic-target.md");
        assert!(graph.nodes[1].rank_signals.semantic > 0.9);
        assert_eq!(graph.ranking_backend, "hybrid_vector_graph");
        assert_eq!(graph.metrics.ranking_backend, "hybrid_vector_graph");
        assert!(graph.metrics.fallback_path.is_none());
    }

    #[test]
    fn graph_metrics_record_semantic_fallback_path() {
        let (_tmp, index) = build_index(&[
            ("/root.md", "---\ntitle: Root\n---\nSee [[Evidence]]."),
            (
                "/evidence.md",
                "---\ntitle: Evidence\n---\nEvaluation proof keeps provenance.",
            ),
        ]);

        let graph = memory_graph_from_index_with_ranking(
            &index,
            MemoryGraphQuery {
                start: "Root".into(),
                query: Some("evaluation proof".into()),
                depth: 1,
                max_nodes: 12,
                max_edges: 16,
                edge_types: Vec::new(),
            },
            MemoryGraphRankingHints::default().with_fallback_path("semantic_unavailable"),
        )
        .unwrap();

        assert_eq!(graph.ranking_backend, "hybrid_lexical_graph");
        assert_eq!(
            graph.metrics.fallback_path.as_deref(),
            Some("semantic_unavailable")
        );
        assert_eq!(graph.metrics.returned_node_count, graph.nodes.len());
        assert_eq!(graph.metrics.returned_edge_count, graph.edges.len());
        assert!(graph.metrics.provenance_preserved);
    }
}
