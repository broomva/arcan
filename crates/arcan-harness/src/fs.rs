use crate::edit::render_hashed_content;
use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolAnnotations, ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use regex::Regex;
use serde_json::json;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct FsPolicy {
    pub workspace_root: PathBuf,
}

impl FsPolicy {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn resolve_existing(&self, candidate: &Path) -> Result<PathBuf, FsPolicyError> {
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.workspace_root.join(candidate)
        };

        let canonical = joined.canonicalize().map_err(|source| FsPolicyError::Io {
            path: joined.clone(),
            source,
        })?;

        self.ensure_within_root(&canonical)?;
        Ok(canonical)
    }

    pub fn resolve_for_write(&self, candidate: &Path) -> Result<PathBuf, FsPolicyError> {
        let joined = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            self.workspace_root.join(candidate)
        };

        // For write, the parent must exist, but the file itself might not.
        let parent = joined
            .parent()
            .ok_or_else(|| FsPolicyError::InvalidPath(joined.display().to_string()))?;

        // If parent doesn't exist, we can't write there (unless we recursively create, which might be a policy choice).
        // For now let's assume parent must exist or we fail.
        let canonical_parent = parent.canonicalize().map_err(|source| FsPolicyError::Io {
            path: parent.to_path_buf(),
            source,
        })?;

        self.ensure_within_root(&canonical_parent)?;

        // Return the path with the canonical parent but the original filename
        Ok(canonical_parent.join(joined.file_name().unwrap()))
    }

    fn ensure_within_root(&self, candidate: &Path) -> Result<(), FsPolicyError> {
        let root = self
            .workspace_root
            .canonicalize()
            .map_err(|source| FsPolicyError::Io {
                path: self.workspace_root.clone(),
                source,
            })?;

        if candidate.starts_with(&root) {
            Ok(())
        } else {
            Err(FsPolicyError::EscapesWorkspace {
                path: candidate.display().to_string(),
                root: root.display().to_string(),
            })
        }
    }
}

#[derive(Debug, Error)]
pub enum FsPolicyError {
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("filesystem IO error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("path {path} escapes workspace root {root}")]
    EscapesWorkspace { path: String, root: String },
}

pub struct ReadFileTool {
    policy: FsPolicy,
}

impl ReadFileTool {
    pub fn new(policy: FsPolicy) -> Self {
        Self { policy }
    }
}

impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Reads a file from the filesystem. Returns content with line numbers and hashes for editing.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" }
                },
                "required": ["path"]
            }),
            title: Some("Read File".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("filesystem".to_string()),
            tags: vec!["fs".to_string(), "read".to_string()],
            timeout_secs: Some(30),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "read_file".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            })?;

        let path = self
            .policy
            .resolve_existing(Path::new(path_str))
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "read_file".to_string(),
                message: e.to_string(),
            })?;

        let content = fs::read_to_string(&path).map_err(|e| CoreError::ToolExecution {
            tool_name: "read_file".to_string(),
            message: format!("Failed to read file: {}", e),
        })?;

        let hashed_content = render_hashed_content(&content);

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "content": hashed_content, "path": path }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

pub struct WriteFileTool {
    policy: FsPolicy,
}

impl WriteFileTool {
    pub fn new(policy: FsPolicy) -> Self {
        Self { policy }
    }
}

impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Writes content to a file, overwriting it completely.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "content": { "type": "string", "description": "Content to write" }
                },
                "required": ["path", "content"]
            }),
            title: Some("Write File".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                destructive: true,
                ..Default::default()
            }),
            category: Some("filesystem".to_string()),
            tags: vec!["fs".to_string(), "write".to_string()],
            timeout_secs: Some(30),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "write_file".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            })?;
        let content = call
            .input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "write_file".to_string(),
                message: "Missing or invalid 'content' argument".to_string(),
            })?;

        let path = self
            .policy
            .resolve_for_write(Path::new(path_str))
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "write_file".to_string(),
                message: e.to_string(),
            })?;

        fs::write(&path, content).map_err(|e| CoreError::ToolExecution {
            tool_name: "write_file".to_string(),
            message: format!("Failed to write file: {}", e),
        })?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "success": true, "path": path }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

pub struct ListDirTool {
    policy: FsPolicy,
}

impl ListDirTool {
    pub fn new(policy: FsPolicy) -> Self {
        Self { policy }
    }
}

impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_dir".to_string(),
            description: "Lists contents of a directory.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the directory" }
                },
                "required": ["path"]
            }),
            title: Some("List Directory".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("filesystem".to_string()),
            tags: vec!["fs".to_string(), "list".to_string()],
            timeout_secs: Some(30),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "list_dir".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            })?;

        let path = self
            .policy
            .resolve_existing(Path::new(path_str))
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "list_dir".to_string(),
                message: e.to_string(),
            })?;

        let entries = fs::read_dir(&path)
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "list_dir".to_string(),
                message: format!("Failed to read dir: {}", e),
            })?
            .filter_map(|entry| {
                entry.ok().map(|e: std::fs::DirEntry| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let kind = if e.path().is_dir() { "dir" } else { "file" };
                    json!({ "name": name, "kind": kind })
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "entries": entries, "path": path }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

pub struct GlobTool {
    policy: FsPolicy,
}

impl GlobTool {
    pub fn new(policy: FsPolicy) -> Self {
        Self { policy }
    }
}

impl Tool for GlobTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "glob".to_string(),
            description: "Search for files matching a glob pattern within the workspace."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Glob pattern (e.g. **/*.rs)" },
                    "path": { "type": "string", "description": "Base directory (optional, defaults to workspace root)" }
                },
                "required": ["pattern"]
            }),
            title: Some("Glob Search".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("filesystem".to_string()),
            tags: vec!["fs".to_string(), "search".to_string()],
            timeout_secs: Some(30),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let pattern = call
            .input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "glob".to_string(),
                message: "Missing or invalid 'pattern' argument".to_string(),
            })?;

        let base_dir = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.policy.workspace_root.clone());

        // Resolve base_dir within workspace
        let base_dir = self
            .policy
            .resolve_existing(Path::new(&base_dir))
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "glob".to_string(),
                message: e.to_string(),
            })?;

        let full_pattern = base_dir.join(pattern).display().to_string();

        let matches: Vec<String> = glob::glob(&full_pattern)
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "glob".to_string(),
                message: format!("Invalid glob pattern: {}", e),
            })?
            .filter_map(Result::ok)
            .filter(|path| {
                // Only include paths within the workspace
                self.policy.resolve_existing(path).is_ok()
            })
            .map(|path| {
                // Return paths relative to workspace root
                path.strip_prefix(&self.policy.workspace_root)
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| path.display().to_string())
            })
            .collect();

        let count = matches.len();

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "matches": matches, "count": count }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

pub struct GrepTool {
    policy: FsPolicy,
}

impl GrepTool {
    pub fn new(policy: FsPolicy) -> Self {
        Self { policy }
    }
}

impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search file contents for a regex pattern within the workspace."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern to search for" },
                    "path": { "type": "string", "description": "Directory to search (optional, defaults to workspace root)" },
                    "glob": { "type": "string", "description": "File glob filter (e.g. *.rs)" },
                    "max_matches": { "type": "integer", "description": "Maximum number of matches to return (default 100)" }
                },
                "required": ["pattern"]
            }),
            title: Some("Grep Search".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("filesystem".to_string()),
            tags: vec!["fs".to_string(), "search".to_string()],
            timeout_secs: Some(60),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let pattern_str = call
            .input
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "grep".to_string(),
                message: "Missing or invalid 'pattern' argument".to_string(),
            })?;

        let regex = Regex::new(pattern_str).map_err(|e| CoreError::ToolExecution {
            tool_name: "grep".to_string(),
            message: format!("Invalid regex pattern: {}", e),
        })?;

        let base_dir = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.policy.workspace_root.clone());

        let base_dir = self
            .policy
            .resolve_existing(Path::new(&base_dir))
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "grep".to_string(),
                message: e.to_string(),
            })?;

        let glob_filter = call.input.get("glob").and_then(|v| v.as_str());

        let max_matches = call
            .input
            .get("max_matches")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(100) as usize;

        let mut matches = Vec::new();

        for entry in walkdir::WalkDir::new(&base_dir)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();

            // Apply glob filter if specified
            if let Some(glob_pat) = glob_filter {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                let glob_pattern =
                    glob::Pattern::new(glob_pat).map_err(|e| CoreError::ToolExecution {
                        tool_name: "grep".to_string(),
                        message: format!("Invalid glob filter: {}", e),
                    })?;
                if !glob_pattern.matches(&file_name) {
                    continue;
                }
            }

            // Skip binary files and large files
            if let Ok(metadata) = path.metadata() {
                if metadata.len() > 10 * 1024 * 1024 {
                    continue; // Skip files > 10MB
                }
            }

            let Ok(file) = fs::File::open(path) else {
                continue;
            };

            let reader = BufReader::new(file);
            for (line_no, line) in reader.lines().enumerate() {
                let Ok(line) = line else {
                    continue; // Skip binary/unreadable lines
                };

                if regex.is_match(&line) {
                    let rel_path = path
                        .strip_prefix(&self.policy.workspace_root)
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|_| path.display().to_string());

                    matches.push(json!({
                        "file": rel_path,
                        "line": line_no + 1,
                        "text": line
                    }));

                    if matches.len() >= max_matches {
                        break;
                    }
                }
            }

            if matches.len() >= max_matches {
                break;
            }
        }

        let count = matches.len();

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "matches": matches, "count": count }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}
