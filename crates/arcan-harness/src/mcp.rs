use arcan_core::error::CoreError;
use arcan_core::protocol::{
    ToolAnnotations as ArcanAnnotations, ToolCall, ToolContent, ToolDefinition, ToolResult,
};
use arcan_core::runtime::{Tool, ToolContext};
use rmcp::model::{CallToolRequestParams, CallToolResult, RawContent, Tool as McpToolDef};
use rmcp::service::{Peer, RoleClient};
use rmcp::transport::TokioChildProcess;
use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::sync::Arc;
use thiserror::Error;
use tokio::process::Command;
use tokio::runtime::Handle;

/// Configuration for connecting to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpTransport {
    Stdio { command: String, args: Vec<String> },
}

/// Bridge: wraps a single MCP tool into Arcan's Tool trait.
pub struct McpTool {
    definition: ToolDefinition,
    peer: Arc<Peer<RoleClient>>,
    mcp_tool_name: String,
    runtime: Handle,
}

impl Tool for McpTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        let arguments = call.input.as_object().cloned();

        let params = CallToolRequestParams {
            meta: None,
            name: Cow::Owned(self.mcp_tool_name.clone()),
            arguments,
            task: None,
        };

        let peer = self.peer.clone();
        let mcp_result: CallToolResult = self
            .runtime
            .block_on(async move { peer.call_tool(params).await })
            .map_err(|e| CoreError::ToolExecution {
                tool_name: call.tool_name.clone(),
                message: format!("MCP call_tool failed: {}", e),
            })?;

        // Convert MCP content to Arcan ToolContent
        let content: Vec<ToolContent> = mcp_result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(text) => Some(ToolContent::Text {
                    text: text.text.clone(),
                }),
                RawContent::Image(img) => Some(ToolContent::Image {
                    data: img.data.clone(),
                    mime_type: img.mime_type.clone(),
                }),
                _ => None,
            })
            .collect();

        // Also build a JSON output for backward compat
        let output = if let Some(structured) = &mcp_result.structured_content {
            structured.clone()
        } else {
            let texts: Vec<String> = mcp_result
                .content
                .iter()
                .filter_map(|c| c.as_text().map(|t| t.text.clone()))
                .collect();
            json!({ "text": texts.join("\n") })
        };

        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output,
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            is_error: mcp_result.is_error.unwrap_or(false),
            state_patch: None,
        })
    }
}

/// Convert an MCP tool definition to Arcan's ToolDefinition.
fn mcp_tool_to_arcan(server_name: &str, mcp_tool: &McpToolDef) -> (ToolDefinition, String) {
    let mcp_name = mcp_tool.name.to_string();
    let arcan_name = format!("mcp_{}_{}", server_name, mcp_name);

    let annotations = mcp_tool.annotations.as_ref().map(|ann| ArcanAnnotations {
        read_only: ann.read_only_hint.unwrap_or(false),
        destructive: ann.destructive_hint.unwrap_or(true),
        idempotent: ann.idempotent_hint.unwrap_or(false),
        open_world: ann.open_world_hint.unwrap_or(true),
        requires_confirmation: false,
    });

    let input_schema = Value::Object(mcp_tool.input_schema.as_ref().clone());

    let output_schema = mcp_tool
        .output_schema
        .as_ref()
        .map(|s| Value::Object(s.as_ref().clone()));

    let def = ToolDefinition {
        name: arcan_name,
        description: mcp_tool.description.as_deref().unwrap_or("").to_string(),
        input_schema,
        title: mcp_tool
            .title
            .clone()
            .or_else(|| mcp_tool.annotations.as_ref().and_then(|a| a.title.clone())),
        output_schema,
        annotations,
        category: Some("mcp".to_string()),
        tags: vec!["mcp".to_string(), server_name.to_string()],
        timeout_secs: Some(60),
    };

    (def, mcp_name)
}

/// A connected MCP client holding the running service.
pub struct McpConnection {
    pub server_name: String,
    pub tools: Vec<McpTool>,
    _service: Box<dyn std::any::Any + Send>,
}

/// Connect to an MCP server via stdio and return all its tools as Arcan Tool objects.
pub async fn connect_mcp_stdio(config: &McpServerConfig) -> Result<McpConnection, McpError> {
    let McpTransport::Stdio {
        ref command,
        ref args,
    } = config.transport;

    let mut cmd = Command::new(command);
    cmd.args(args);

    let transport = TokioChildProcess::new(cmd).map_err(|e| McpError::Connection(e.to_string()))?;

    let service = ().serve(transport).await.map_err(|e| McpError::Initialize(e.to_string()))?;

    // List tools
    let tools_result = service
        .list_tools(None)
        .await
        .map_err(|e| McpError::ListTools(e.to_string()))?;

    let runtime = Handle::current();
    let peer: Arc<Peer<RoleClient>> = Arc::new(service.peer().clone());

    let mcp_tools: Vec<McpTool> = tools_result
        .tools
        .iter()
        .map(|mcp_tool| {
            let (definition, mcp_tool_name) = mcp_tool_to_arcan(&config.name, mcp_tool);
            McpTool {
                definition,
                peer: peer.clone(),
                mcp_tool_name,
                runtime: runtime.clone(),
            }
        })
        .collect();

    Ok(McpConnection {
        server_name: config.name.clone(),
        tools: mcp_tools,
        _service: Box::new(service),
    })
}

#[derive(Debug, Error)]
pub enum McpError {
    #[error("MCP connection failed: {0}")]
    Connection(String),
    #[error("MCP initialization failed: {0}")]
    Initialize(String),
    #[error("MCP tools/list failed: {0}")]
    ListTools(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_tool_to_arcan_conversion() {
        use rmcp::model::{Tool as McpToolDef, ToolAnnotations as McpAnnotations};
        use std::sync::Arc;

        let mcp_tool = McpToolDef {
            name: "read_file".into(),
            title: Some("Read File".to_string()),
            description: Some("Reads a file from disk".into()),
            input_schema: Arc::new(
                serde_json::from_value(json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
                    },
                    "required": ["path"]
                }))
                .unwrap(),
            ),
            output_schema: None,
            annotations: Some(McpAnnotations {
                title: None,
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                open_world_hint: Some(false),
            }),
            execution: None,
            icons: None,
            meta: None,
        };

        let (def, original_name) = mcp_tool_to_arcan("test-server", &mcp_tool);

        assert_eq!(def.name, "mcp_test-server_read_file");
        assert_eq!(original_name, "read_file");
        assert_eq!(def.description, "Reads a file from disk");
        assert_eq!(def.title, Some("Read File".to_string()));
        assert_eq!(def.category, Some("mcp".to_string()));
        assert!(def.tags.contains(&"mcp".to_string()));
        assert!(def.tags.contains(&"test-server".to_string()));

        let ann = def.annotations.unwrap();
        assert!(ann.read_only);
        assert!(!ann.destructive);
        assert!(ann.idempotent);
        assert!(!ann.open_world);
    }

    #[test]
    fn mcp_tool_without_annotations() {
        use rmcp::model::Tool as McpToolDef;
        use std::sync::Arc;

        let mcp_tool = McpToolDef {
            name: "simple".into(),
            title: None,
            description: Some("A simple tool".into()),
            input_schema: Arc::new(serde_json::from_value(json!({"type": "object"})).unwrap()),
            output_schema: None,
            annotations: None,
            execution: None,
            icons: None,
            meta: None,
        };

        let (def, _) = mcp_tool_to_arcan("srv", &mcp_tool);
        assert_eq!(def.name, "mcp_srv_simple");
        assert!(def.annotations.is_none());
        assert!(def.title.is_none());
    }
}
