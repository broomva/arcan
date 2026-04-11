//! Basic Agent Example
//!
//! Demonstrates the core Agent OS primitives without any API keys:
//! 1. Implement a mock Provider (LLM backend)
//! 2. Register tools
//! 3. Send a message and get a response
//!
//! Run with: `cargo run -p arcan-core --example basic_agent`

use arcan_core::error::CoreError;
use arcan_core::protocol::{
    ChatMessage, ModelDirective, ModelStopReason, ModelTurn, ToolCall, ToolDefinition, ToolResult,
};
use arcan_core::runtime::{Provider, ProviderRequest, Tool, ToolContext, ToolRegistry};

// Step 1: Define a mock provider that echoes messages.
// In production, you'd use AnthropicProvider or OpenAiCompatibleProvider.
struct EchoProvider;

impl Provider for EchoProvider {
    fn name(&self) -> &str {
        "echo"
    }

    fn complete(&self, request: &ProviderRequest) -> Result<ModelTurn, CoreError> {
        let last_message = request
            .messages
            .last()
            .map(|m| m.content.as_str())
            .unwrap_or("(empty)");

        // If the user asks about time, call our custom tool
        if last_message.contains("time") {
            return Ok(ModelTurn {
                directives: vec![ModelDirective::ToolCall {
                    call: ToolCall {
                        call_id: "call_1".to_string(),
                        tool_name: "current_time".to_string(),
                        input: serde_json::json!({}),
                    },
                }],
                stop_reason: ModelStopReason::ToolUse,
                usage: None,
                telemetry: None,
            });
        }

        // Otherwise, echo the message back
        Ok(ModelTurn {
            directives: vec![ModelDirective::Text {
                delta: format!("You said: {last_message}"),
            }],
            stop_reason: ModelStopReason::EndTurn,
            usage: None,
            telemetry: None,
        })
    }
}

// Step 2: Define a simple tool.
// Tools are how agents interact with the world.
struct CurrentTimeTool;

impl Tool for CurrentTimeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "current_time".to_string(),
            description: "Returns the current UTC time".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            title: None,
            output_schema: None,
            annotations: None,
            category: None,
            tags: Vec::new(),
            timeout_secs: None,
        }
    }

    fn execute(&self, call: &ToolCall, _ctx: &ToolContext) -> Result<ToolResult, CoreError> {
        // In a real tool, this would call std::time or chrono
        Ok(ToolResult {
            call_id: call.call_id.clone(),
            tool_name: call.tool_name.clone(),
            output: serde_json::json!("2026-04-10T12:00:00Z (mock time)"),
            content: None,
            is_error: false,
            state_patch: None,
        })
    }
}

fn main() {
    // Step 3: Create the provider and tool registry
    let provider = EchoProvider;
    let mut registry = ToolRegistry::default();
    registry.register(CurrentTimeTool);

    println!("Life Agent OS - Basic Example");
    println!("Provider: {}", provider.name());
    println!(
        "Tools: {:?}",
        registry
            .definitions()
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );
    println!();

    // Step 4: Send a message and get a response
    let messages = vec![
        ChatMessage::system("You are a helpful assistant."),
        ChatMessage::user("Hello, what time is it?"),
    ];
    let tool_defs = registry.definitions();

    let request = ProviderRequest {
        run_id: "example-run".to_string(),
        session_id: "example-session".to_string(),
        iteration: 0,
        messages: messages.clone(),
        tools: tool_defs,
        max_tokens: None,
        state: Default::default(),
    };

    let turn = provider.complete(&request).expect("provider call failed");

    // Step 5: Process the response
    for directive in &turn.directives {
        match directive {
            ModelDirective::Text { delta } => {
                println!("Assistant: {delta}");
            }
            ModelDirective::ToolCall { call } => {
                println!("Tool call: {} ({})", call.tool_name, call.call_id);

                // Execute the tool
                let ctx = ToolContext {
                    run_id: "example-run".to_string(),
                    session_id: "example-session".to_string(),
                    iteration: 0,
                };
                let result = registry
                    .get(&call.tool_name)
                    .expect("tool not found")
                    .execute(call, &ctx)
                    .expect("tool execution failed");

                println!("Tool result: {}", result.output);
            }
            _ => {}
        }
    }

    println!();
    println!("Done! See docs/QUICKSTART.md for next steps.");
}
