//! Bounded memory graph retrieval over `lago-knowledge`.
//!
//! The graph is a derived view over markdown memory artifacts. This adapter
//! shapes Lago traversal primitives into compact, provenance-preserving payloads
//! that Arcan can expose as an agent tool.

use std::collections::{HashMap, HashSet};
use std::path::Path;

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
    pub depth: usize,
    pub max_nodes: usize,
    pub max_edges: usize,
    pub edge_types: Vec<String>,
}

impl MemoryGraphQuery {
    pub fn new(start: impl Into<String>) -> Self {
        Self {
            start: start.into(),
            depth: DEFAULT_GRAPH_DEPTH,
            max_nodes: DEFAULT_MAX_NODES,
            max_edges: DEFAULT_MAX_EDGES,
            edge_types: Vec::new(),
        }
    }

    pub fn bounded(mut self) -> Self {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryGraphNode {
    pub node_id: String,
    pub node_type: String,
    pub title: String,
    pub summary: String,
    pub source_ref: String,
    pub depth: usize,
    pub outgoing_links: Vec<String>,
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
    let (index, _store) = build_index_from_dir(memory_dir)?;
    memory_graph_from_index(&index, query)
}

/// Build a graph response from an already-built knowledge index.
pub fn memory_graph_from_index(
    index: &KnowledgeIndex,
    query: MemoryGraphQuery,
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

    let mut traversal = index.traverse(
        &start_note.path,
        effective_depth,
        query.max_nodes.saturating_add(1),
    );
    let nodes_overflowed = traversal.len() > query.max_nodes;
    traversal.truncate(query.max_nodes);

    let returned_paths: HashSet<&str> = traversal.iter().map(|node| node.path.as_str()).collect();

    let graph_edges = if query.includes_references() {
        graph_edges(index, &traversal, &returned_paths, query.max_edges)
    } else {
        GraphEdges::default()
    };
    let edges_overflowed = graph_edges.overflowed;
    let mut outgoing_links_by_source = graph_edges.outgoing_links_by_source;

    let nodes = traversal
        .iter()
        .filter_map(|node| {
            index.get_note(&node.path).map(|note| MemoryGraphNode {
                node_id: note.path.clone(),
                node_type: note_type(note),
                title: note_title(note),
                summary: note_summary(note),
                source_ref: note.path.clone(),
                depth: node.depth,
                outgoing_links: outgoing_links_by_source
                    .remove(&note.path)
                    .unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();

    let truncated = nodes_overflowed || edges_overflowed;
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
    })
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
}
