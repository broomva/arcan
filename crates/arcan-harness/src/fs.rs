use crate::edit::render_hashed_content;
use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use serde_json::json;
use std::fs;
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

        let canonical = joined
            .canonicalize()
            .map_err(|source| FsPolicyError::Io {
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
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call.input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            CoreError::ToolExecution {
                tool_name: "read_file".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            }
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
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call.input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            CoreError::ToolExecution {
                tool_name: "write_file".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            }
        })?;
        let content = call.input.get("content").and_then(|v| v.as_str()).ok_or_else(|| {
            CoreError::ToolExecution {
                tool_name: "write_file".to_string(),
                message: "Missing or invalid 'content' argument".to_string(),
            }
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
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call.input.get("path").and_then(|v| v.as_str()).ok_or_else(|| {
            CoreError::ToolExecution {
                tool_name: "list_dir".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            }
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
            state_patch: None,
        })
    }
}
