use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolAnnotations, ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use serde_json::json;
use std::fs;
use std::path::PathBuf;

pub struct ReadMemoryTool {
    memory_dir: PathBuf,
}

impl ReadMemoryTool {
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }
}

impl Tool for ReadMemoryTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_memory".to_string(),
            description: "Read the agent's persistent memory file by key.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Memory key (e.g. 'global', 'session', or custom name)" }
                },
                "required": ["key"]
            }),
            title: Some("Read Memory".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only: true,
                idempotent: true,
                ..Default::default()
            }),
            category: Some("memory".to_string()),
            tags: vec!["memory".to_string(), "read".to_string()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let key = call
            .input
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "read_memory".to_string(),
                message: "Missing or invalid 'key' argument".to_string(),
            })?;

        validate_memory_key(key).map_err(|msg| CoreError::ToolExecution {
            tool_name: "read_memory".to_string(),
            message: msg,
        })?;

        let file_path = self.memory_dir.join(format!("{}.md", key));

        if file_path.exists() {
            let content = fs::read_to_string(&file_path).map_err(|e| CoreError::ToolExecution {
                tool_name: "read_memory".to_string(),
                message: format!("Failed to read memory file: {}", e),
            })?;

            Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: json!({ "content": content, "exists": true, "path": file_path }),
                content: None,
                is_error: false,
                state_patch: None,
            })
        } else {
            Ok(ToolResult {
                call_id: call.call_id.clone(),
                tool_name: call.tool_name.clone(),
                output: json!({ "content": null, "exists": false, "path": file_path }),
                content: None,
                is_error: false,
                state_patch: None,
            })
        }
    }
}

pub struct WriteMemoryTool {
    memory_dir: PathBuf,
}

impl WriteMemoryTool {
    pub fn new(memory_dir: PathBuf) -> Self {
        Self { memory_dir }
    }
}

impl Tool for WriteMemoryTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_memory".to_string(),
            description: "Write to the agent's persistent memory file by key.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string", "description": "Memory key (e.g. 'global', 'session', or custom name)" },
                    "content": { "type": "string", "description": "Markdown content to write" }
                },
                "required": ["key", "content"]
            }),
            title: Some("Write Memory".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                destructive: true,
                ..Default::default()
            }),
            category: Some("memory".to_string()),
            tags: vec!["memory".to_string(), "write".to_string()],
            timeout_secs: Some(10),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let key = call
            .input
            .get("key")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "write_memory".to_string(),
                message: "Missing or invalid 'key' argument".to_string(),
            })?;

        let content = call
            .input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "write_memory".to_string(),
                message: "Missing or invalid 'content' argument".to_string(),
            })?;

        validate_memory_key(key).map_err(|msg| CoreError::ToolExecution {
            tool_name: "write_memory".to_string(),
            message: msg,
        })?;

        // Ensure memory directory exists
        fs::create_dir_all(&self.memory_dir).map_err(|e| CoreError::ToolExecution {
            tool_name: "write_memory".to_string(),
            message: format!("Failed to create memory directory: {}", e),
        })?;

        let file_path = self.memory_dir.join(format!("{}.md", key));

        fs::write(&file_path, content).map_err(|e| CoreError::ToolExecution {
            tool_name: "write_memory".to_string(),
            message: format!("Failed to write memory file: {}", e),
        })?;

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "success": true, "path": file_path }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

/// Validate that a memory key is safe (alphanumeric, hyphens, underscores).
fn validate_memory_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("Memory key cannot be empty".to_string());
    }
    if key.len() > 64 {
        return Err("Memory key too long (max 64 characters)".to_string());
    }
    if !key
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Memory key must contain only alphanumeric characters, hyphens, and underscores"
                .to_string(),
        );
    }
    if key.starts_with('.') || key.contains("..") {
        return Err("Memory key cannot start with '.' or contain '..'".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcan_core::runtime::ToolContext;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_ctx() -> ToolContext {
        ToolContext {
            run_id: "test-run".to_string(),
            session_id: "test".to_string(),
            iteration: 0,
        }
    }

    fn make_call(name: &str, input: serde_json::Value) -> ToolCall {
        ToolCall {
            call_id: "test-call".to_string(),
            tool_name: name.to_string(),
            input,
        }
    }

    #[test]
    fn write_then_read_memory() {
        let dir = TempDir::new().unwrap();
        let write_tool = WriteMemoryTool::new(dir.path().to_path_buf());
        let read_tool = ReadMemoryTool::new(dir.path().to_path_buf());
        let ctx = make_ctx();

        // Write
        let call = make_call(
            "write_memory",
            json!({"key": "test-notes", "content": "# Notes\nSome important info."}),
        );
        let result = write_tool.execute(&call, &ctx).unwrap();
        assert_eq!(result.output["success"], true);

        // Read back
        let call = make_call("read_memory", json!({"key": "test-notes"}));
        let result = read_tool.execute(&call, &ctx).unwrap();
        assert_eq!(result.output["exists"], true);
        assert_eq!(result.output["content"], "# Notes\nSome important info.");
    }

    #[test]
    fn read_nonexistent_memory() {
        let dir = TempDir::new().unwrap();
        let read_tool = ReadMemoryTool::new(dir.path().to_path_buf());
        let ctx = make_ctx();

        let call = make_call("read_memory", json!({"key": "nonexistent"}));
        let result = read_tool.execute(&call, &ctx).unwrap();
        assert_eq!(result.output["exists"], false);
        assert!(result.output["content"].is_null());
    }

    #[test]
    fn invalid_memory_key_rejected() {
        assert!(validate_memory_key("").is_err());
        assert!(validate_memory_key("../escape").is_err());
        assert!(validate_memory_key(".hidden").is_err());
        assert!(validate_memory_key("has spaces").is_err());
        assert!(validate_memory_key("has/slash").is_err());

        assert!(validate_memory_key("valid-key").is_ok());
        assert!(validate_memory_key("valid_key").is_ok());
        assert!(validate_memory_key("key123").is_ok());
    }

    #[test]
    fn write_creates_directory() {
        let dir = TempDir::new().unwrap();
        let memory_dir = dir.path().join("memory");
        // Directory doesn't exist yet
        assert!(!memory_dir.exists());

        let write_tool = WriteMemoryTool::new(memory_dir.clone());
        let ctx = make_ctx();

        let call = make_call(
            "write_memory",
            json!({"key": "auto-created", "content": "hello"}),
        );
        let result = write_tool.execute(&call, &ctx).unwrap();
        assert_eq!(result.output["success"], true);
        assert!(memory_dir.exists());
    }

    #[test]
    fn overwrite_memory() {
        let dir = TempDir::new().unwrap();
        let write_tool = WriteMemoryTool::new(dir.path().to_path_buf());
        let read_tool = ReadMemoryTool::new(dir.path().to_path_buf());
        let ctx = make_ctx();

        // Write initial
        let call = make_call(
            "write_memory",
            json!({"key": "overwrite-test", "content": "version 1"}),
        );
        write_tool.execute(&call, &ctx).unwrap();

        // Overwrite
        let call = make_call(
            "write_memory",
            json!({"key": "overwrite-test", "content": "version 2"}),
        );
        write_tool.execute(&call, &ctx).unwrap();

        // Read back
        let call = make_call("read_memory", json!({"key": "overwrite-test"}));
        let result = read_tool.execute(&call, &ctx).unwrap();
        assert_eq!(result.output["content"], "version 2");
    }
}
