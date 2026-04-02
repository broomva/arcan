//! Agent-driven memory retrieval tools (BRO-417).
//!
//! Five tools the agent calls proactively to manage its own memory:
//! - `memory_search` — keyword search across memory files
//! - `memory_browse` — list memories by tier/type
//! - `memory_recent` — last N memories by modification time
//! - `memory_offload` — save content to episodic memory
//! - `memory_forget` — mark a memory as low importance

use aios_protocol::tool::{
    Tool, ToolAnnotations, ToolCall, ToolContext, ToolDefinition, ToolError, ToolResult,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

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

        let query_lower = query.to_lowercase();
        let keywords: Vec<&str> = query_lower.split_whitespace().collect();

        if keywords.is_empty() {
            return Err(ToolError::InvalidInput {
                message: "Query cannot be empty".into(),
            });
        }

        let mut matches = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.memory_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "md") {
                    if let Ok(content) = fs::read_to_string(&path) {
                        let content_lower = content.to_lowercase();
                        let hit_count = keywords
                            .iter()
                            .filter(|kw| content_lower.contains(*kw))
                            .count();

                        if hit_count > 0 {
                            let file_name = path
                                .file_stem()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string();

                            // Extract relevant excerpt (first matching line + context)
                            let excerpt = extract_excerpt(&content, &keywords, 3);

                            matches.push(json!({
                                "file": file_name,
                                "relevance": hit_count,
                                "excerpt": excerpt,
                            }));
                        }
                    }
                }
            }
        }

        // Sort by relevance (most keyword hits first)
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
                if path.extension().is_some_and(|e| e == "md") {
                    if let Ok(content) = fs::read_to_string(&path) {
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
                        if let Some(ref ft) = filter_tier {
                            if tier.to_lowercase() != *ft {
                                continue;
                            }
                        }
                        if let Some(ref fty) = filter_type {
                            if mem_type.to_lowercase() != *fty {
                                continue;
                            }
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
                if path.extension().is_some_and(|e| e == "md") {
                    if let Ok(meta) = fs::metadata(&path) {
                        if let Ok(modified) = meta.modified() {
                            files.push((path, modified));
                        }
                    }
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
    use tempfile::TempDir;

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

        let tool = MemorySearchTool::new(&dir.path().to_path_buf());
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

        let tool = MemorySearchTool::new(&dir.path().to_path_buf());
        let call = make_call("memory_search", json!({"query": "nonexistent keyword xyz"}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        assert_eq!(result.output["total"], 0);
    }

    #[test]
    fn search_rejects_empty_query() {
        let dir = TempDir::new().unwrap();
        let tool = MemorySearchTool::new(&dir.path().to_path_buf());
        let call = make_call("memory_search", json!({"query": "   "}));
        let err = tool.execute(&call, &make_ctx()).unwrap_err();
        assert!(matches!(err, ToolError::InvalidInput { .. }));
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

        let tool = MemoryBrowseTool::new(&dir.path().to_path_buf());

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

        let tool = MemoryRecentTool::new(&dir.path().to_path_buf());
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

        let tool = MemoryRecentTool::new(&dir.path().to_path_buf());
        let call = make_call("memory_recent", json!({}));
        let result = tool.execute(&call, &make_ctx()).unwrap();

        let memories = result.output["memories"].as_array().unwrap();
        assert_eq!(memories.len(), 5);
    }

    // ── MemoryOffloadTool tests ──

    #[test]
    fn offload_creates_file_with_frontmatter() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryOffloadTool::new(&dir.path().to_path_buf());

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
        let tool = MemoryOffloadTool::new(&dir.path().to_path_buf());

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
        let tool = MemoryOffloadTool::new(&dir.path().to_path_buf());

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

        let tool = MemoryForgetTool::new(&dir.path().to_path_buf());
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

        let tool = MemoryForgetTool::new(&dir.path().to_path_buf());
        let call = make_call("memory_forget", json!({"key": "no-importance"}));
        tool.execute(&call, &make_ctx()).unwrap();

        let content = fs::read_to_string(dir.path().join("no-importance.md")).unwrap();
        assert!(content.contains("importance: 0.1"));
    }

    #[test]
    fn forget_nonexistent_file_returns_error() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryForgetTool::new(&dir.path().to_path_buf());
        let call = make_call("memory_forget", json!({"key": "nonexistent"}));
        let err = tool.execute(&call, &make_ctx()).unwrap_err();
        assert!(matches!(err, ToolError::ExecutionFailed { .. }));
    }

    #[test]
    fn forget_rejects_invalid_key() {
        let dir = TempDir::new().unwrap();
        let tool = MemoryForgetTool::new(&dir.path().to_path_buf());

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
            Box::new(MemoryBrowseTool::new(&path)),
            Box::new(MemoryRecentTool::new(&path)),
            Box::new(MemoryOffloadTool::new(&path)),
            Box::new(MemoryForgetTool::new(&path)),
        ];

        let expected_names = [
            "memory_search",
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
