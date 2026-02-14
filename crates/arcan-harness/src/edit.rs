use blake3::Hasher;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fs::FsPolicy;
use arcan_core::error::CoreError;
use arcan_core::protocol::{ToolAnnotations, ToolCall, ToolDefinition, ToolResult};
use arcan_core::runtime::{Tool, ToolContext};
use serde_json::json;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct HashedLine {
    pub line_no: usize,
    pub tag: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TaggedEditOp {
    ReplaceLine { tag: String, new_text: String },
    InsertAfterTag { tag: String, new_text: String },
    DeleteLine { tag: String },
}

#[derive(Debug, Clone)]
struct EditableLine {
    anchor_tag: Option<String>,
    text: String,
}

pub fn hash_lines(content: &str) -> Vec<HashedLine> {
    content
        .lines()
        .enumerate()
        .map(|(idx, line)| HashedLine {
            line_no: idx + 1,
            tag: line_tag(idx + 1, line),
            text: line.to_string(),
        })
        .collect()
}

pub fn render_hashed_content(content: &str) -> String {
    hash_lines(content)
        .into_iter()
        .map(|line| format!("{:>4} {} | {}", line.line_no, line.tag, line.text))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn apply_tagged_edits(content: &str, ops: &[TaggedEditOp]) -> Result<String, EditError> {
    let initial_lines = hash_lines(content);

    if initial_lines.is_empty() && !ops.is_empty() {
        return Err(EditError::EmptyContent);
    }

    let mut editable = initial_lines
        .iter()
        .map(|line| EditableLine {
            anchor_tag: Some(line.tag.clone()),
            text: line.text.clone(),
        })
        .collect::<Vec<_>>();

    for op in ops {
        match op {
            TaggedEditOp::ReplaceLine { tag, new_text } => {
                let idx = find_anchor_index(&editable, tag)?;
                editable[idx].text = new_text.clone();
            }
            TaggedEditOp::InsertAfterTag { tag, new_text } => {
                let idx = find_anchor_index(&editable, tag)?;
                editable.insert(
                    idx + 1,
                    EditableLine {
                        anchor_tag: None,
                        text: new_text.clone(),
                    },
                );
            }
            TaggedEditOp::DeleteLine { tag } => {
                let idx = find_anchor_index(&editable, tag)?;
                editable.remove(idx);
            }
        }
    }

    Ok(editable
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n"))
}

fn find_anchor_index(lines: &[EditableLine], tag: &str) -> Result<usize, EditError> {
    lines
        .iter()
        .position(|line| line.anchor_tag.as_deref() == Some(tag))
        .ok_or_else(|| EditError::StaleTag(tag.to_string()))
}

fn line_tag(line_no: usize, text: &str) -> String {
    let mut hasher = Hasher::new();
    hasher.update(line_no.to_string().as_bytes());
    hasher.update(b":");
    hasher.update(text.as_bytes());
    let digest = hasher.finalize().to_hex().to_string();
    digest.chars().take(8).collect()
}

#[derive(Debug, Error)]
pub enum EditError {
    #[error("input content is empty; tags are required before edit")]
    EmptyContent,
    #[error("tag is stale or missing in current content: {0}")]
    StaleTag(String),
}

// --- EditFileTool: filesystem tool wrapping the hashline editor ---

pub struct EditFileTool {
    policy: FsPolicy,
}

impl EditFileTool {
    pub fn new(policy: FsPolicy) -> Self {
        Self { policy }
    }
}

impl Tool for EditFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit_file".to_string(),
            description: "Edits a file using line tags. Operations: replace_line, insert_after_tag, delete_line.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to the file" },
                    "ops": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "oneOf": [
                                { "properties": { "op": { "const": "replace_line" }, "tag": { "type": "string" }, "new_text": { "type": "string" } }, "required": ["op", "tag", "new_text"] },
                                { "properties": { "op": { "const": "insert_after_tag" }, "tag": { "type": "string" }, "new_text": { "type": "string" } }, "required": ["op", "tag", "new_text"] },
                                { "properties": { "op": { "const": "delete_line" }, "tag": { "type": "string" } }, "required": ["op", "tag"] }
                            ]
                        }
                    }
                },
                "required": ["path", "ops"]
            }),
            title: Some("Edit File".to_string()),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                destructive: true,
                ..Default::default()
            }),
            category: Some("filesystem".to_string()),
            tags: vec!["fs".to_string(), "edit".to_string()],
            timeout_secs: Some(30),
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let path_str = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "edit_file".to_string(),
                message: "Missing or invalid 'path' argument".to_string(),
            })?;

        let ops_value = call
            .input
            .get("ops")
            .ok_or_else(|| CoreError::ToolExecution {
                tool_name: "edit_file".to_string(),
                message: "Missing 'ops' argument".to_string(),
            })?;

        let ops: Vec<TaggedEditOp> =
            serde_json::from_value(ops_value.clone()).map_err(|e| CoreError::ToolExecution {
                tool_name: "edit_file".to_string(),
                message: format!("Invalid 'ops' format: {}", e),
            })?;

        let path = self
            .policy
            .resolve_for_write(Path::new(path_str))
            .map_err(|e| CoreError::ToolExecution {
                tool_name: "edit_file".to_string(),
                message: e.to_string(),
            })?;

        // Read current content
        let content = fs::read_to_string(&path).map_err(|e| CoreError::ToolExecution {
            tool_name: "edit_file".to_string(),
            message: format!("Failed to read file: {}", e),
        })?;

        // Apply edits
        let new_content =
            apply_tagged_edits(&content, &ops).map_err(|e| CoreError::ToolExecution {
                tool_name: "edit_file".to_string(),
                message: format!("Edit failed: {}", e),
            })?;

        // Write back
        fs::write(&path, &new_content).map_err(|e| CoreError::ToolExecution {
            tool_name: "edit_file".to_string(),
            message: format!("Failed to write file: {}", e),
        })?;

        // Return hashed content of the NEW file
        let hashed_content = render_hashed_content(&new_content);

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: json!({ "success": true, "content": hashed_content, "path": path }),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{apply_tagged_edits, hash_lines, TaggedEditOp};

    #[test]
    fn replaces_line_by_tag() {
        let input = "a\nb\nc";
        let lines = hash_lines(input);

        let output = apply_tagged_edits(
            input,
            &[TaggedEditOp::ReplaceLine {
                tag: lines[1].tag.clone(),
                new_text: "B".to_string(),
            }],
        )
        .expect("replace succeeds");

        assert_eq!(output, "a\nB\nc");
    }

    #[test]
    fn stale_tag_fails() {
        let err = apply_tagged_edits(
            "a",
            &[TaggedEditOp::DeleteLine {
                tag: "missing".to_string(),
            }],
        )
        .expect_err("stale tag should fail");

        assert!(format!("{err}").contains("stale or missing"));
    }

    #[test]
    fn inserts_after_tag() {
        let input = "line1\nline2\nline3";
        let lines = hash_lines(input);

        let output = apply_tagged_edits(
            input,
            &[TaggedEditOp::InsertAfterTag {
                tag: lines[1].tag.clone(),
                new_text: "inserted".to_string(),
            }],
        )
        .expect("insert succeeds");

        assert_eq!(output, "line1\nline2\ninserted\nline3");
    }

    #[test]
    fn deletes_line_by_tag() {
        let input = "a\nb\nc";
        let lines = hash_lines(input);

        let output = apply_tagged_edits(
            input,
            &[TaggedEditOp::DeleteLine {
                tag: lines[1].tag.clone(),
            }],
        )
        .expect("delete succeeds");

        assert_eq!(output, "a\nc");
    }

    #[test]
    fn multiple_operations_applied_sequentially() {
        let input = "first\nsecond\nthird";
        let lines = hash_lines(input);

        let output = apply_tagged_edits(
            input,
            &[
                TaggedEditOp::ReplaceLine {
                    tag: lines[0].tag.clone(),
                    new_text: "FIRST".to_string(),
                },
                TaggedEditOp::DeleteLine {
                    tag: lines[2].tag.clone(),
                },
            ],
        )
        .expect("multi-op succeeds");

        assert_eq!(output, "FIRST\nsecond");
    }

    #[test]
    fn empty_content_with_ops_fails() {
        let err = apply_tagged_edits(
            "",
            &[TaggedEditOp::DeleteLine {
                tag: "any".to_string(),
            }],
        )
        .expect_err("should fail on empty");

        assert!(format!("{err}").contains("empty"));
    }

    #[test]
    fn hash_lines_produces_unique_tags() {
        let input = "same\nsame\nsame";
        let lines = hash_lines(input);

        // Each line should have a unique tag (because line_no differs)
        assert_ne!(lines[0].tag, lines[1].tag);
        assert_ne!(lines[1].tag, lines[2].tag);
    }

    #[test]
    fn render_hashed_content_has_line_numbers() {
        let input = "hello\nworld";
        let rendered = super::render_hashed_content(input);
        assert!(rendered.contains("   1 "));
        assert!(rendered.contains("   2 "));
        assert!(rendered.contains("hello"));
        assert!(rendered.contains("world"));
    }
}
