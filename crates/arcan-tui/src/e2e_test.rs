//! End-to-end TUI experience tests.
//!
//! These tests simulate what a real user would see: they create an App with a
//! mock client, type messages via key events, inject agent responses, render
//! frames to a `TestBackend`, and assert on the rendered terminal output.
//!
//! Each test captures the full "screen" at key moments, making it possible to
//! validate the TUI experience without running an interactive terminal.

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::event::TuiEvent;
    use crate::mock_client::MockAgentClient;
    use crate::models::state::ConnectionStatus;
    use crate::models::ui_block::UiBlock;
    use arcan_core::protocol::{AgentEvent, RunStopReason, ToolCall, ToolResultSummary};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::sync::Arc;

    fn press(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    /// Type a string into the app's input bar character by character.
    async fn type_and_submit(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.input_bar.input(press(KeyCode::Char(ch)));
        }
        app.handle_key(press(KeyCode::Enter)).await;
    }

    fn make_app() -> (App, Arc<MockAgentClient>) {
        let client = Arc::new(MockAgentClient::new("e2e-test"));
        let app = App::new(client.clone());
        (app, client)
    }

    /// Render the app and return the full terminal content as a string.
    fn render_frame(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| crate::ui::draw(f, app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let mut rows = Vec::new();
        for y in 0..height {
            let mut row = String::new();
            for x in 0..width {
                row.push_str(buffer[(x, y)].symbol());
            }
            rows.push(row.trim_end().to_string());
        }
        rows.join("\n")
    }

    // ── Multi-turn conversation ────────────────────────────────────────────

    #[tokio::test]
    async fn multi_turn_conversation_renders_correctly() {
        let (mut app, _client) = make_app();

        // Frame 0: initial state — should show connection info
        let frame0 = render_frame(&mut app, 100, 30);
        assert!(
            frame0.contains("e2e-test"),
            "initial frame should show session ID"
        );
        assert!(frame0.contains("Session Log"), "should show chat log title");

        // Turn 1: user types a message
        type_and_submit(&mut app, "What is 2+2?").await;
        let frame1 = render_frame(&mut app, 100, 30);
        assert!(frame1.contains("You:"), "should show user message label");
        assert!(
            frame1.contains("What is 2+2?"),
            "should show user message text"
        );
        assert!(
            frame1.contains("Thinking"),
            "should show thinking/busy indicator: {frame1}"
        );

        // Agent responds with RunStarted + text deltas + RunFinished
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });

        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "2+2 equals 4.".into(),
        });

        // Frame during streaming — should show partial text
        let frame_streaming = render_frame(&mut app, 100, 30);
        assert!(
            frame_streaming.contains("2+2 equals 4."),
            "should show streaming text: {frame_streaming}"
        );

        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("2+2 equals 4.".into()),
            usage: None,
        });

        assert!(!app.state.is_busy);

        let frame2 = render_frame(&mut app, 100, 30);
        assert!(
            frame2.contains("Assistant:"),
            "should show assistant label: {frame2}"
        );
        assert!(
            frame2.contains("2+2 equals 4."),
            "should show assistant response"
        );
        // Thinking indicator should be gone
        assert!(
            !frame2.contains("Thinking"),
            "thinking should disappear after RunFinished: {frame2}"
        );

        // Turn 2: user sends another message
        type_and_submit(&mut app, "What about 3+3?").await;
        let frame3 = render_frame(&mut app, 100, 30);
        assert!(
            frame3.contains("What about 3+3?"),
            "should show second user message"
        );
        // Previous assistant message should still be visible
        assert!(
            frame3.contains("2+2 equals 4."),
            "should keep previous assistant response in history"
        );

        // Agent responds to turn 2
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r2".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r2".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "3+3 equals 6.".into(),
        });
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r2".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("3+3 equals 6.".into()),
            usage: None,
        });

        let frame4 = render_frame(&mut app, 100, 30);
        // Both turns should be visible
        assert!(
            frame4.contains("What is 2+2?"),
            "turn 1 user message should be in history"
        );
        assert!(
            frame4.contains("2+2 equals 4."),
            "turn 1 assistant response should be in history"
        );
        assert!(
            frame4.contains("What about 3+3?"),
            "turn 2 user message should be in history"
        );
        assert!(
            frame4.contains("3+3 equals 6."),
            "turn 2 assistant response should be in history"
        );
    }

    // ── Tool execution rendering ───────────────────────────────────────────

    #[tokio::test]
    async fn tool_execution_renders_inline() {
        let (mut app, _) = make_app();

        // Send a message
        type_and_submit(&mut app, "Read the file foo.txt").await;

        // Agent uses a tool
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });

        app.state.apply_event(AgentEvent::ToolCallRequested {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            call: ToolCall {
                call_id: "call-1".into(),
                tool_name: "fs.read".into(),
                input: serde_json::json!({"path": "foo.txt"}),
            },
        });

        let frame_tool_running = render_frame(&mut app, 100, 30);
        assert!(
            frame_tool_running.contains("fs.read"),
            "should show tool name while running: {frame_tool_running}"
        );

        // Tool completes
        app.state.apply_event(AgentEvent::ToolCallCompleted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            result: ToolResultSummary {
                call_id: "call-1".into(),
                tool_name: "fs.read".into(),
                output: serde_json::json!("file contents here"),
            },
        });

        let frame_tool_done = render_frame(&mut app, 100, 30);
        assert!(
            frame_tool_done.contains("fs.read"),
            "should still show tool name after completion"
        );

        // Agent finishes with text
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "The file contains: file contents here".into(),
        });
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("The file contains: file contents here".into()),
            usage: None,
        });

        let frame_final = render_frame(&mut app, 100, 30);
        assert!(
            frame_final.contains("file contents here"),
            "should show assistant response after tool use"
        );
        assert!(!app.state.is_busy);
    }

    // ── Error handling ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn run_error_shows_error_and_allows_retry() {
        let (mut app, _) = make_app();

        type_and_submit(&mut app, "hello").await;

        // Agent errors
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "anthropic".into(),
            max_iterations: 10,
        });
        app.state.apply_event(AgentEvent::RunErrored {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            error: "rate_limit_error: Too Many Requests".into(),
        });

        let frame_error = render_frame(&mut app, 100, 30);
        assert!(
            frame_error.contains("rate_limit_error"),
            "should display error to user: {frame_error}"
        );
        assert!(!app.state.is_busy, "should not be busy after error");

        // User can retry
        type_and_submit(&mut app, "hello again").await;
        assert!(app.state.is_busy, "should be busy after retry");

        let frame_retry = render_frame(&mut app, 100, 30);
        assert!(
            frame_retry.contains("hello again"),
            "retry message should be visible"
        );
    }

    #[tokio::test]
    async fn api_error_json_shows_formatted_error() {
        let (mut app, _) = make_app();

        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "anthropic".into(),
            max_iterations: 10,
        });

        // Simulate an API error that comes through as text (Anthropic format)
        let error_json =
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#;
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: error_json.into(),
        });
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some(error_json.into()),
            usage: None,
        });

        let frame = render_frame(&mut app, 100, 30);
        // Should extract and display the error message, not raw JSON
        assert!(
            frame.contains("API Error") || frame.contains("overloaded_error"),
            "should show formatted API error, not raw JSON: {frame}"
        );
    }

    // ── Slash commands ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn slash_help_shows_all_commands() {
        let (mut app, _) = make_app();

        type_and_submit(&mut app, "/help").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(frame.contains("/clear"), "help should list /clear");
        assert!(frame.contains("/model"), "help should list /model");
        assert!(frame.contains("/sessions"), "help should list /sessions");
        assert!(frame.contains("/provider"), "help should list /provider");
    }

    #[tokio::test]
    async fn slash_clear_resets_chat() {
        let (mut app, _) = make_app();

        // Add some content
        type_and_submit(&mut app, "hello").await;
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "world".into(),
        });
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("world".into()),
            usage: None,
        });

        let frame_before = render_frame(&mut app, 100, 30);
        assert!(frame_before.contains("hello"));
        assert!(frame_before.contains("world"));

        // Clear
        type_and_submit(&mut app, "/clear").await;

        let frame_after = render_frame(&mut app, 100, 30);
        assert!(
            !frame_after.contains("hello"),
            "clear should remove messages"
        );
        assert!(
            !frame_after.contains("world"),
            "clear should remove assistant messages"
        );
    }

    // ── Keyboard navigation ────────────────────────────────────────────────

    #[tokio::test]
    async fn escape_exits_gracefully() {
        let (mut app, _) = make_app();
        assert!(!app.should_quit);

        app.handle_key(press(KeyCode::Esc)).await;
        assert!(app.should_quit, "Esc should set should_quit");
    }

    #[tokio::test]
    async fn ctrl_c_exits() {
        let (mut app, _) = make_app();
        assert!(!app.should_quit);

        app.handle_key(ctrl(KeyCode::Char('c'))).await;
        assert!(app.should_quit, "Ctrl+C should exit");
    }

    #[tokio::test]
    async fn input_history_navigation() {
        let (mut app, _) = make_app();

        // Type and submit two messages
        type_and_submit(&mut app, "first message").await;
        type_and_submit(&mut app, "second message").await;

        // Press Up to navigate history
        app.handle_key(press(KeyCode::Up)).await;
        assert_eq!(
            app.input_bar.text(),
            "second message",
            "Up should recall last message"
        );

        app.handle_key(press(KeyCode::Up)).await;
        assert_eq!(
            app.input_bar.text(),
            "first message",
            "Up again should recall first message"
        );

        app.handle_key(press(KeyCode::Down)).await;
        assert_eq!(
            app.input_bar.text(),
            "second message",
            "Down should go back to second message"
        );
    }

    // ── Connection status ──────────────────────────────────────────────────

    #[tokio::test]
    async fn connection_status_reflected_in_render() {
        let (mut app, _) = make_app();

        // Status bar uses Unicode indicators:
        // ● (U+25CF) = Connected, ○ (U+25CB) = Disconnected, ◌ (U+25CC) = Connecting

        // Connecting state (default)
        app.state.connection_status = ConnectionStatus::Connecting;
        let frame_connecting = render_frame(&mut app, 100, 30);
        assert!(
            frame_connecting.contains('\u{25cc}'),
            "should show ◌ (connecting) indicator: {frame_connecting}"
        );

        // Connected state
        app.state.connection_status = ConnectionStatus::Connected;
        let frame_connected = render_frame(&mut app, 100, 30);
        assert!(
            frame_connected.contains('\u{25cf}'),
            "should show ● (connected) indicator: {frame_connected}"
        );

        // Disconnected state
        app.state.connection_status = ConnectionStatus::Disconnected;
        let frame_disconnected = render_frame(&mut app, 100, 30);
        assert!(
            frame_disconnected.contains('\u{25cb}'),
            "should show ○ (disconnected) indicator: {frame_disconnected}"
        );
    }

    // ── Session browser ────────────────────────────────────────────────────

    #[tokio::test]
    async fn sessions_command_opens_panel() {
        let (mut app, _) = make_app();
        assert!(!app.show_panels);

        type_and_submit(&mut app, "/sessions").await;

        assert!(app.show_panels, "should open panels");

        let frame = render_frame(&mut app, 120, 30);
        assert!(
            frame.contains("Sessions") || frame.contains("e2e-test"),
            "should show session browser panel: {frame}"
        );
    }

    // ── State inspector ────────────────────────────────────────────────────

    #[tokio::test]
    async fn state_command_opens_inspector() {
        let (mut app, _) = make_app();

        type_and_submit(&mut app, "/state").await;

        assert!(app.show_panels, "should open panels");

        let frame = render_frame(&mut app, 120, 30);
        assert!(
            frame.contains("State") || frame.contains("Explore") || frame.contains("progress"),
            "should show state inspector: {frame}"
        );
    }

    // ── Autocomplete ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn autocomplete_appears_on_slash() {
        let (mut app, _) = make_app();

        // Type "/" to trigger autocomplete
        app.input_bar.input(press(KeyCode::Char('/')));
        app.autocomplete.update(app.input_bar.text());

        assert!(app.autocomplete.active, "autocomplete should activate on /");
        assert!(
            !app.autocomplete.suggestions.is_empty(),
            "should have suggestions"
        );

        let frame = render_frame(&mut app, 100, 30);
        // The autocomplete popup should render on top
        assert!(
            frame.contains("clear") || frame.contains("help") || frame.contains("model"),
            "autocomplete popup should show command suggestions: {frame}"
        );
    }

    #[tokio::test]
    async fn autocomplete_filters_on_typing() {
        let (mut app, _) = make_app();

        // Type "/mo" to filter to /model
        for ch in "/mo".chars() {
            app.input_bar.input(press(KeyCode::Char(ch)));
        }
        app.autocomplete.update(app.input_bar.text());

        assert!(app.autocomplete.active);
        assert!(
            app.autocomplete
                .suggestions
                .iter()
                .any(|s| s.name.contains("model")),
            "should suggest /model for '/mo'"
        );
    }

    // ── Narrow terminal rendering ──────────────────────────────────────────

    #[tokio::test]
    async fn renders_on_very_narrow_terminal() {
        let (mut app, _) = make_app();

        type_and_submit(&mut app, "hello world this is a long message").await;
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "This is a response that should wrap on narrow terminals".into(),
        });
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: None,
            usage: None,
        });

        // 30 cols, 15 rows — should not panic
        let frame = render_frame(&mut app, 30, 15);
        assert!(
            !frame.is_empty(),
            "should render something on narrow terminal"
        );
    }

    #[tokio::test]
    async fn renders_on_very_tall_terminal() {
        let (mut app, _) = make_app();

        // Single message on a very tall terminal
        type_and_submit(&mut app, "hello").await;

        let frame = render_frame(&mut app, 80, 60);
        assert!(frame.contains("hello"), "should render on tall terminal");
    }

    // ── Approval flow ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn approval_requested_shows_banner() {
        let (mut app, _) = make_app();

        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });

        app.state.apply_event(AgentEvent::ApprovalRequested {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            approval_id: "ap-1".into(),
            call_id: "call-1".into(),
            tool_name: "shell.exec".into(),
            arguments: serde_json::json!({"cmd": "rm -rf /tmp/test"}),
            risk: "high".into(),
        });

        assert!(app.state.pending_approval.is_some());
        assert!(!app.state.is_busy, "approval pauses the run");

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("shell.exec") || frame.contains("Approval"),
            "should show approval banner: {frame}"
        );
    }

    #[tokio::test]
    async fn approval_resolved_resumes_run() {
        let (mut app, _) = make_app();

        // Setup: approval pending
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });
        app.state.apply_event(AgentEvent::ApprovalRequested {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            approval_id: "ap-1".into(),
            call_id: "call-1".into(),
            tool_name: "shell.exec".into(),
            arguments: serde_json::json!({}),
            risk: "high".into(),
        });
        assert!(app.state.pending_approval.is_some());

        // Resolve
        app.state.apply_event(AgentEvent::ApprovalResolved {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            approval_id: "ap-1".into(),
            decision: "approved".into(),
            reason: None,
        });

        assert!(app.state.pending_approval.is_none());
        assert!(app.state.is_busy, "should resume run after approval");

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("approved"),
            "should show approval decision: {frame}"
        );
    }

    // ── Streaming text display ─────────────────────────────────────────────

    #[tokio::test]
    async fn streaming_text_accumulates_deltas() {
        let (mut app, _) = make_app();

        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });

        // Stream text in small chunks
        for word in ["Hello", " ", "world", "!", " This", " is", " streaming."] {
            app.state.apply_event(AgentEvent::TextDelta {
                run_id: "r1".into(),
                session_id: "e2e-test".into(),
                iteration: 0,
                delta: word.into(),
            });
        }

        assert_eq!(
            app.state.streaming_text,
            Some("Hello world! This is streaming.".into())
        );

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Hello world!"),
            "streaming text should be visible: {frame}"
        );
    }

    // ── Provider switching ─────────────────────────────────────────────────

    #[tokio::test]
    async fn model_set_command_works() {
        let (mut app, _) = make_app();

        type_and_submit(&mut app, "/model set ollama llama3.2").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("switched") || frame.contains("ollama"),
            "should confirm provider switch: {frame}"
        );
    }

    // ── Many messages stress test ──────────────────────────────────────────

    #[tokio::test]
    async fn many_messages_render_without_panic() {
        let (mut app, _) = make_app();
        app.state.blocks.clear();

        // Add 100 messages to stress test rendering
        for i in 0..100 {
            app.state.blocks.push(UiBlock::HumanMessage {
                text: format!("Message {i}: some content"),
                timestamp: chrono::Utc::now(),
            });
            app.state.blocks.push(UiBlock::AssistantMessage {
                text: format!("Response {i}: here is the answer"),
                timestamp: chrono::Utc::now(),
            });
        }

        // Should not panic on any reasonable terminal size
        let frame = render_frame(&mut app, 80, 24);
        assert!(!frame.is_empty());

        // Scroll to top and back
        let max = app.state.scroll.total_lines;
        app.state.scroll.scroll_up(max);
        let frame_top = render_frame(&mut app, 80, 24);
        assert!(!frame_top.is_empty());

        app.state.scroll.scroll_to_bottom();
        let frame_bottom = render_frame(&mut app, 80, 24);
        assert!(!frame_bottom.is_empty());
    }

    // ── Deduplication ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn duplicate_assistant_messages_are_deduplicated() {
        let (mut app, _) = make_app();
        let initial_blocks = app.state.blocks.len();

        // Simulate streaming text + RunFinished with same text
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "Hello world".into(),
        });
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("Hello world".into()),
            usage: None,
        });

        // Should only have one assistant message, not two
        let assistant_count = app.state.blocks[initial_blocks..]
            .iter()
            .filter(|b| matches!(b, UiBlock::AssistantMessage { .. }))
            .count();
        assert_eq!(
            assistant_count, 1,
            "should have exactly one assistant message, not duplicate"
        );
    }

    // ── Tool call failure rendering ────────────────────────────────────────

    #[tokio::test]
    async fn failed_tool_call_shows_error() {
        let (mut app, _) = make_app();

        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 10,
        });

        app.state.apply_event(AgentEvent::ToolCallRequested {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            call: ToolCall {
                call_id: "call-1".into(),
                tool_name: "shell.exec".into(),
                input: serde_json::json!({"cmd": "whoami"}),
            },
        });

        app.state.apply_event(AgentEvent::ToolCallFailed {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            call_id: "call-1".into(),
            tool_name: "shell.exec".into(),
            error: "Permission denied".into(),
        });

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("shell.exec"),
            "should show failed tool name: {frame}"
        );
        // The tool panel should indicate error status
        let has_tool_error = app.state.blocks.iter().any(|b| {
            matches!(
                b,
                UiBlock::ToolExecution {
                    status: crate::models::ui_block::ToolStatus::Error(_),
                    ..
                }
            )
        });
        assert!(has_tool_error, "tool should be in error state");
    }

    // ── Daemon event format (TextDelta vs AssistantTextDelta) ─────────────

    /// The daemon persists `TextDelta` events to the journal (not `AssistantTextDelta`).
    /// When the TUI reads from the SSE stream, it receives these persisted events.
    /// This test verifies the TUI handles them correctly.
    #[tokio::test]
    async fn handles_persisted_text_delta_from_daemon() {
        use crate::client::agent_event_from_protocol_record;
        use aios_protocol::{
            BranchId as ProtocolBranchId, EventKind as ProtocolEventKind,
            EventRecord as ProtocolEventRecord, SessionId as ProtocolSessionId,
        };

        // Simulate a TextDelta event from the daemon journal (persisted format)
        let record = ProtocolEventRecord::new(
            ProtocolSessionId::from_string("test"),
            ProtocolBranchId::main(),
            9,
            ProtocolEventKind::TextDelta {
                delta: "Hello from daemon".to_string(),
                index: Some(0),
            },
        );

        let event = agent_event_from_protocol_record(&record);
        assert!(
            event.is_some(),
            "TextDelta from daemon journal must be handled (not ignored)"
        );

        match event.unwrap() {
            AgentEvent::TextDelta { delta, .. } => {
                assert_eq!(delta, "Hello from daemon");
            }
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    /// Full conversation flow using daemon-format events (persisted TextDelta + RunFinished without final_answer).
    #[tokio::test]
    async fn daemon_format_conversation_renders_text() {
        let (mut app, _) = make_app();

        type_and_submit(&mut app, "hello").await;

        // Simulate daemon event sequence: RunStarted → TextDelta → RunFinished(no final_answer)
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            provider: "mock".into(),
            max_iterations: 1,
        });
        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            delta: "Echo: hello".into(),
        });
        // Daemon sends RunFinished without final_answer
        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: None, // Daemon mock doesn't set this
            usage: None,
        });

        assert!(!app.state.is_busy);

        // The streaming_text should have been flushed into an AssistantMessage block
        let has_response = app.state.blocks.iter().any(
            |b| matches!(b, UiBlock::AssistantMessage { text, .. } if text.contains("Echo: hello")),
        );
        assert!(
            has_response,
            "should show assistant response from TextDelta even when final_answer is None"
        );

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Echo: hello"),
            "rendered frame should show response: {frame}"
        );
    }

    // ── Context compaction event ───────────────────────────────────────────

    #[tokio::test]
    async fn context_compaction_is_handled() {
        let (mut app, _) = make_app();

        // Context compaction shouldn't crash or affect state negatively
        app.state.apply_event(AgentEvent::ContextCompacted {
            run_id: "r1".into(),
            session_id: "e2e-test".into(),
            iteration: 0,
            dropped_count: 5,
            tokens_before: 10000,
            tokens_after: 5000,
        });

        // Should handle gracefully (event goes to _ => {} catch-all)
        let frame = render_frame(&mut app, 100, 30);
        assert!(!frame.is_empty(), "should render after compaction event");
    }

    // ── Full lifecycle: daemon → SSE → TUI (with mock daemon) ──────────────

    #[tokio::test]
    async fn full_lifecycle_with_mock_events() {
        let client = Arc::new(MockAgentClient::new("lifecycle-test"));

        // Pre-load events that simulate a complete agent turn
        client
            .set_events(vec![
                AgentEvent::RunStarted {
                    run_id: "r1".into(),
                    session_id: "lifecycle-test".into(),
                    provider: "mock".into(),
                    max_iterations: 10,
                },
                AgentEvent::TextDelta {
                    run_id: "r1".into(),
                    session_id: "lifecycle-test".into(),
                    iteration: 0,
                    delta: "I'm a mock agent.".into(),
                },
                AgentEvent::RunFinished {
                    run_id: "r1".into(),
                    session_id: "lifecycle-test".into(),
                    reason: RunStopReason::Completed,
                    total_iterations: 1,
                    final_answer: Some("I'm a mock agent.".into()),
                    usage: None,
                },
            ])
            .await;

        let mut app = App::new(client.clone() as Arc<dyn crate::client::AgentClientPort>);

        // Process pre-loaded events by polling the event channel
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match tokio::time::timeout_at(deadline, app.events.recv()).await {
                Ok(Some(TuiEvent::Network(event))) => {
                    app.state.connection_status = ConnectionStatus::Connected;
                    app.state.apply_event(event);
                }
                Ok(Some(TuiEvent::ConnectionLost)) => break,
                Ok(Some(_)) => {} // tick, etc.
                Ok(None) | Err(_) => break,
            }
        }

        // Should have processed all events
        assert!(!app.state.is_busy, "should not be busy after RunFinished");
        let has_assistant = app.state.blocks.iter().any(
            |b| matches!(b, UiBlock::AssistantMessage { text, .. } if text.contains("mock agent")),
        );
        assert!(
            has_assistant,
            "should have assistant message from pre-loaded events"
        );

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("mock agent"),
            "rendered frame should show agent response: {frame}"
        );
    }

    // ── New daemon-proxy commands ───���───────────────────────────────────────

    #[tokio::test]
    async fn autonomic_command_shows_ruling() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/autonomic").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Autonomic") || frame.contains("Breathe"),
            "should show autonomic ruling: {frame}"
        );
    }

    #[tokio::test]
    async fn context_command_shows_usage() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/context").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Context") || frame.contains("tokens"),
            "should show context info: {frame}"
        );
    }

    #[tokio::test]
    async fn cost_command_shows_budget() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/cost").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Cost") || frame.contains("remaining"),
            "should show cost info: {frame}"
        );
    }

    #[tokio::test]
    async fn compact_command_shows_stub() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/compact").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Compact") || frame.contains("not yet implemented"),
            "should show compact stub message: {frame}"
        );
    }

    #[tokio::test]
    async fn config_command_shows_configuration() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/config").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Configuration") || frame.contains("provider"),
            "should show configuration info: {frame}"
        );
    }

    #[tokio::test]
    async fn memory_command_shows_memory_info() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/memory").await;

        let frame = render_frame(&mut app, 100, 30);
        // Memory directory may or may not exist, but command should not panic
        // and should show either "Memory" or file listing
        assert!(
            frame.contains("Memory") || frame.contains("memory"),
            "should show memory info: {frame}"
        );
    }

    #[tokio::test]
    async fn status_command_shows_summary() {
        let (mut app, _) = make_app();
        type_and_submit(&mut app, "/status").await;

        let frame = render_frame(&mut app, 100, 30);
        assert!(
            frame.contains("Status") || frame.contains("Session"),
            "should show status summary: {frame}"
        );
    }
}
