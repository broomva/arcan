//! Knowledge context assembly for Arcan sessions.
//!
//! Builds a `ContextBlock::Retrieval` from the knowledge index on session
//! bootstrap. This is the Life-native equivalent of the wake-up protocol —
//! no hooks needed, it's in the architecture.

use arcan_core::context_compiler::{ContextBlock, ContextBlockKind};
use lago_core::ManifestEntry;
use lago_knowledge::KnowledgeIndex;
use lago_store::BlobStore;
use tracing::{debug, info, warn};

/// Build a knowledge retrieval context block from a directory of `.md` files.
///
/// Returns `None` if no knowledge is available (empty dir, build failure).
/// Never fails — errors are logged and the session continues without knowledge.
pub fn build_knowledge_block(
    wiki_dir: &std::path::Path,
    token_budget: usize,
) -> Option<ContextBlock> {
    let md_files = collect_md_files(wiki_dir);
    if md_files.is_empty() {
        debug!(dir = %wiki_dir.display(), "no .md files found, skipping knowledge block");
        return None;
    }

    // Build temp blob store + index
    let blob_dir = wiki_dir.join(".lago-blobs");
    if let Err(e) = std::fs::create_dir_all(&blob_dir) {
        warn!(error = %e, "failed to create blob dir for knowledge index");
        return None;
    }

    let store = match BlobStore::open(&blob_dir) {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "failed to open blob store for knowledge index");
            return None;
        }
    };

    let mut entries = Vec::new();
    for file in &md_files {
        let content = match std::fs::read(file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let hash = match store.put(&content) {
            Ok(h) => h,
            Err(_) => continue,
        };
        let rel_path = file
            .strip_prefix(wiki_dir)
            .unwrap_or(file)
            .to_string_lossy();
        entries.push(ManifestEntry {
            path: format!("/{rel_path}"),
            blob_hash: hash,
            size_bytes: content.len() as u64,
            content_type: Some("text/markdown".to_string()),
            updated_at: 0,
        });
    }

    let index = match KnowledgeIndex::build(&entries, &store) {
        Ok(idx) => idx,
        Err(e) => {
            warn!(error = %e, "failed to build knowledge index");
            return None;
        }
    };

    let note_count = index.len();
    if note_count == 0 {
        return None;
    }

    // Assemble L1: top notes by frontmatter score
    let mut scored: Vec<(&str, &str, i64)> = Vec::new();
    for note in index.notes().values() {
        let score = note
            .frontmatter
            .get("scoring")
            .and_then(|s| s.get("raw_score"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let title = note
            .frontmatter
            .get("core_claim")
            .and_then(|v| v.as_str())
            .or_else(|| note.frontmatter.get("title").and_then(|v| v.as_str()))
            .unwrap_or(&note.name);
        scored.push((&note.name, title, score));
    }

    scored.sort_by(|a, b| b.2.cmp(&a.2));

    let mut content = format!("## Knowledge Graph ({note_count} entities)\n\n");
    let mut tokens_used = content.len() / 4;

    for (name, claim, score) in &scored {
        let line = format!("- {name} | {claim} | score: {score}\n");
        let est = line.len() / 4;
        if tokens_used + est > token_budget {
            break;
        }
        content.push_str(&line);
        tokens_used += est;
    }

    info!(
        notes = note_count,
        tokens = tokens_used,
        "knowledge context block assembled"
    );

    Some(ContextBlock {
        kind: ContextBlockKind::Retrieval,
        content,
        priority: 150,
    })
}

/// Build a `KnowledgeIndex` from a directory, returning it with the backing store.
///
/// The caller must keep the `BlobStore` alive as long as the index is used.
pub fn build_index_from_dir(
    wiki_dir: &std::path::Path,
) -> Result<(KnowledgeIndex, BlobStore), lago_knowledge::KnowledgeError> {
    let blob_dir = wiki_dir.join(".lago-blobs");
    std::fs::create_dir_all(&blob_dir)
        .map_err(|e| lago_knowledge::KnowledgeError::Store(e.to_string()))?;
    let store = BlobStore::open(&blob_dir)
        .map_err(|e| lago_knowledge::KnowledgeError::Store(e.to_string()))?;

    let md_files = collect_md_files(wiki_dir);
    let mut entries = Vec::new();

    for file in &md_files {
        let content = std::fs::read(file)
            .map_err(|e| lago_knowledge::KnowledgeError::Store(e.to_string()))?;
        let hash = store
            .put(&content)
            .map_err(|e| lago_knowledge::KnowledgeError::Store(e.to_string()))?;
        let rel_path = file
            .strip_prefix(wiki_dir)
            .unwrap_or(file)
            .to_string_lossy();
        entries.push(ManifestEntry {
            path: format!("/{rel_path}"),
            blob_hash: hash,
            size_bytes: content.len() as u64,
            content_type: Some("text/markdown".to_string()),
            updated_at: 0,
        });
    }

    let index = KnowledgeIndex::build(&entries, &store)?;
    Ok((index, store))
}

/// Recursively collect `.md` files, skipping hidden dirs and build artifacts.
fn collect_md_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return files,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if name.starts_with('.') || name == "node_modules" || name == "target" {
            continue;
        }
        if path.is_dir() {
            files.extend(collect_md_files(&path));
        } else if path.extension().is_some_and(|e| e == "md") {
            files.push(path);
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn build_block_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let block = build_knowledge_block(tmp.path(), 600);
        assert!(block.is_none());
    }

    #[test]
    fn build_block_with_notes() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("test.md"),
            "---\ntitle: Test\ncore_claim: Testing works\nscoring:\n  raw_score: 7\n---\n# Test\n\nContent.",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("other.md"),
            "---\ntitle: Other\ncore_claim: Other claim\nscoring:\n  raw_score: 3\n---\n# Other\n\nMore.",
        )
        .unwrap();

        let block = build_knowledge_block(tmp.path(), 600).unwrap();
        assert_eq!(block.kind, ContextBlockKind::Retrieval);
        assert!(block.content.contains("2 entities"));
        // Higher-scored note should appear first
        assert!(block.content.find("test").unwrap() < block.content.find("other").unwrap());
    }

    #[test]
    fn build_block_respects_budget() {
        let tmp = TempDir::new().unwrap();
        for i in 0..50 {
            std::fs::write(
                tmp.path().join(format!("note-{i}.md")),
                format!("---\ntitle: Note {i}\nscoring:\n  raw_score: {i}\n---\n# Note {i}\n\nContent for note {i}."),
            )
            .unwrap();
        }

        let block = build_knowledge_block(tmp.path(), 100).unwrap();
        // Should be well under 100 tokens worth
        assert!(block.content.len() < 500); // 100 tokens * ~4 chars + header
    }

    #[test]
    fn build_index_from_dir_works() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("a.md"), "# A\n\nContent.").unwrap();
        let (index, _store) = build_index_from_dir(tmp.path()).unwrap();
        assert_eq!(index.len(), 1);
    }
}
