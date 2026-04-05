//! Bridge adapter: wraps any `aios_protocol::tool::Tool` as an `arcan_core::runtime::Tool`.
//!
//! This lets Praxis canonical tools (which implement the aiOS `Tool` trait) be
//! registered in Arcan's `ToolRegistry` without modifying either side.

use arcan_core::error::CoreError;
use arcan_core::protocol as arcan_proto;
use arcan_core::runtime as arcan_rt;

use aios_protocol::tool as proto_tool;

/// Adapter that wraps an `aios_protocol::tool::Tool` impl so it satisfies
/// `arcan_core::runtime::Tool`.
pub struct PraxisToolBridge<T> {
    inner: T,
}

impl<T> PraxisToolBridge<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

// ── Type conversion helpers ──────────────────────────────────────────

fn proto_def_to_arcan(def: proto_tool::ToolDefinition) -> arcan_proto::ToolDefinition {
    arcan_proto::ToolDefinition {
        name: def.name,
        description: def.description,
        input_schema: def.input_schema,
        title: def.title,
        output_schema: def.output_schema,
        annotations: def.annotations.map(|a| arcan_proto::ToolAnnotations {
            read_only: a.read_only,
            destructive: a.destructive,
            idempotent: a.idempotent,
            open_world: a.open_world,
            requires_confirmation: a.requires_confirmation,
        }),
        category: def.category,
        tags: def.tags,
        timeout_secs: def.timeout_secs,
    }
}

fn arcan_call_to_proto(call: &arcan_proto::ToolCall) -> proto_tool::ToolCall {
    proto_tool::ToolCall {
        call_id: call.call_id.clone(),
        tool_name: call.tool_name.clone(),
        input: call.input.clone(),
        requested_capabilities: Vec::new(),
    }
}

fn arcan_ctx_to_proto(ctx: &arcan_rt::ToolContext) -> proto_tool::ToolContext {
    proto_tool::ToolContext {
        run_id: ctx.run_id.clone(),
        session_id: ctx.session_id.clone(),
        iteration: ctx.iteration,
    }
}

fn proto_result_to_arcan(result: proto_tool::ToolResult) -> arcan_proto::ToolResult {
    arcan_proto::ToolResult {
        call_id: result.call_id,
        tool_name: result.tool_name,
        output: result.output,
        content: result.content.map(|blocks| {
            blocks
                .into_iter()
                .map(|c| match c {
                    proto_tool::ToolContent::Text { text } => {
                        arcan_proto::ToolContent::Text { text }
                    }
                    proto_tool::ToolContent::Image { data, mime_type } => {
                        arcan_proto::ToolContent::Image { data, mime_type }
                    }
                    proto_tool::ToolContent::Json { value } => {
                        arcan_proto::ToolContent::Json { value }
                    }
                })
                .collect()
        }),
        is_error: result.is_error,
        state_patch: None,
    }
}

fn proto_err_to_core(err: proto_tool::ToolError) -> CoreError {
    match err {
        proto_tool::ToolError::NotFound { tool_name } => CoreError::ToolNotFound { tool_name },
        proto_tool::ToolError::ExecutionFailed { tool_name, message } => {
            CoreError::ToolExecution { tool_name, message }
        }
        other => CoreError::ToolExecution {
            tool_name: String::new(),
            message: other.to_string(),
        },
    }
}

// ── Trait impl ───────────────────────────────────────────────────────

impl<T: proto_tool::Tool + Send + Sync> arcan_rt::Tool for PraxisToolBridge<T> {
    fn definition(&self) -> arcan_proto::ToolDefinition {
        proto_def_to_arcan(self.inner.definition())
    }

    fn execute(
        &self,
        call: &arcan_proto::ToolCall,
        ctx: &arcan_rt::ToolContext,
    ) -> Result<arcan_proto::ToolResult, CoreError> {
        let proto_call = arcan_call_to_proto(call);
        let proto_ctx = arcan_ctx_to_proto(ctx);

        self.inner
            .execute(&proto_call, &proto_ctx)
            .map(proto_result_to_arcan)
            .map_err(proto_err_to_core)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A minimal praxis-style tool for testing the bridge.
    struct EchoProtoTool;

    impl proto_tool::Tool for EchoProtoTool {
        fn definition(&self) -> proto_tool::ToolDefinition {
            proto_tool::ToolDefinition {
                name: "echo".into(),
                description: "Echoes input".into(),
                input_schema: json!({"type": "object"}),
                title: None,
                output_schema: None,
                annotations: Some(proto_tool::ToolAnnotations {
                    read_only: true,
                    idempotent: true,
                    ..Default::default()
                }),
                category: Some("test".into()),
                tags: vec!["test".into()],
                timeout_secs: Some(10),
            }
        }

        fn execute(
            &self,
            call: &proto_tool::ToolCall,
            _ctx: &proto_tool::ToolContext,
        ) -> Result<proto_tool::ToolResult, proto_tool::ToolError> {
            Ok(proto_tool::ToolResult::text(
                &call.call_id,
                &call.tool_name,
                "echoed",
            ))
        }
    }

    struct FailProtoTool;

    impl proto_tool::Tool for FailProtoTool {
        fn definition(&self) -> proto_tool::ToolDefinition {
            proto_tool::ToolDefinition {
                name: "fail".into(),
                description: "Always fails".into(),
                input_schema: json!({"type": "object"}),
                title: None,
                output_schema: None,
                annotations: None,
                category: None,
                tags: vec![],
                timeout_secs: None,
            }
        }

        fn execute(
            &self,
            call: &proto_tool::ToolCall,
            _ctx: &proto_tool::ToolContext,
        ) -> Result<proto_tool::ToolResult, proto_tool::ToolError> {
            Err(proto_tool::ToolError::ExecutionFailed {
                tool_name: call.tool_name.clone(),
                message: "always fails".into(),
            })
        }
    }

    fn arcan_ctx() -> arcan_rt::ToolContext {
        arcan_rt::ToolContext {
            run_id: "run-1".into(),
            session_id: "sess-1".into(),
            iteration: 1,
        }
    }

    fn arcan_call(name: &str) -> arcan_proto::ToolCall {
        arcan_proto::ToolCall {
            call_id: "call-1".into(),
            tool_name: name.into(),
            input: json!({}),
        }
    }

    #[test]
    fn bridge_definition_converts_all_fields() {
        let bridge = PraxisToolBridge::new(EchoProtoTool);
        let def: arcan_proto::ToolDefinition = arcan_rt::Tool::definition(&bridge);

        assert_eq!(def.name, "echo");
        assert_eq!(def.description, "Echoes input");
        assert_eq!(def.category.as_deref(), Some("test"));
        assert_eq!(def.tags, vec!["test".to_string()]);
        assert_eq!(def.timeout_secs, Some(10));

        let ann = def.annotations.unwrap();
        assert!(ann.read_only);
        assert!(ann.idempotent);
        assert!(!ann.destructive);
    }

    #[test]
    fn bridge_execute_success() {
        let bridge = PraxisToolBridge::new(EchoProtoTool);
        let result = arcan_rt::Tool::execute(&bridge, &arcan_call("echo"), &arcan_ctx()).unwrap();

        assert_eq!(result.call_id, "call-1");
        assert_eq!(result.tool_name, "echo");
        assert!(!result.is_error);
        assert!(result.state_patch.is_none());
    }

    #[test]
    fn bridge_execute_error_maps_to_core_error() {
        let bridge = PraxisToolBridge::new(FailProtoTool);
        let err = arcan_rt::Tool::execute(&bridge, &arcan_call("fail"), &arcan_ctx()).unwrap_err();

        match err {
            CoreError::ToolExecution { tool_name, message } => {
                assert_eq!(tool_name, "fail");
                assert!(message.contains("always fails"));
            }
            other => panic!("expected ToolExecution, got {other:?}"),
        }
    }

    #[test]
    fn bridge_tool_not_found_maps_correctly() {
        let err = proto_err_to_core(proto_tool::ToolError::NotFound {
            tool_name: "ghost".into(),
        });
        match err {
            CoreError::ToolNotFound { tool_name } => assert_eq!(tool_name, "ghost"),
            other => panic!("expected ToolNotFound, got {other:?}"),
        }
    }

    #[test]
    fn bridge_can_register_in_arcan_registry() {
        let mut registry = arcan_rt::ToolRegistry::default();
        registry.register(PraxisToolBridge::new(EchoProtoTool));

        assert_eq!(registry.definitions().len(), 1);
        assert!(registry.get("echo").is_some());
    }

    #[test]
    fn bridge_content_blocks_convert() {
        let proto_result = proto_tool::ToolResult {
            call_id: "c1".into(),
            tool_name: "t1".into(),
            output: json!("hello"),
            content: Some(vec![
                proto_tool::ToolContent::Text {
                    text: "hello".into(),
                },
                proto_tool::ToolContent::Json {
                    value: json!({"key": "val"}),
                },
                proto_tool::ToolContent::Image {
                    data: "base64".into(),
                    mime_type: "image/png".into(),
                },
            ]),
            is_error: false,
        };

        let arcan_result = proto_result_to_arcan(proto_result);
        let blocks = arcan_result.content.unwrap();
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[0], arcan_proto::ToolContent::Text { text } if text == "hello"));
        assert!(matches!(&blocks[1], arcan_proto::ToolContent::Json { .. }));
        assert!(matches!(&blocks[2], arcan_proto::ToolContent::Image { .. }));
    }
}
