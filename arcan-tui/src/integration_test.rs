#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::event::TuiEvent;
    use crate::mock_client::MockAgentClient;
    use crate::models::state::ConnectionStatus;
    use crate::models::ui_block::UiBlock;
    use crate::widgets::markdown::MarkdownRenderer;
    use arcan_core::protocol::{AgentEvent, RunStopReason};
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

    /// Type a string into the input bar character by character.
    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.input_bar.input(press(KeyCode::Char(ch)));
        }
    }

    fn make_app() -> (App, Arc<MockAgentClient>) {
        let client = Arc::new(MockAgentClient::new("test-session"));
        let app = App::new(client.clone());
        (app, client)
    }

    // ── Startup ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn startup_shows_system_alert_with_connection_info() {
        let (app, _) = make_app();
        assert!(!app.state.blocks.is_empty());
        match &app.state.blocks[0] {
            UiBlock::SystemAlert { text, .. } => {
                assert!(
                    text.contains("localhost:3000"),
                    "startup alert should mention base URL: {text}"
                );
                assert!(
                    text.contains("test-session"),
                    "startup alert should mention session ID: {text}"
                );
            }
            other => panic!("expected SystemAlert, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn startup_sets_session_id() {
        let (app, _) = make_app();
        assert_eq!(app.state.session_id, Some("test-session".to_string()));
    }

    // ── Slash commands ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn help_command_adds_system_alert() {
        let (mut app, _) = make_app();
        let initial_blocks = app.state.blocks.len();

        type_text(&mut app, "/help");
        app.handle_key(press(KeyCode::Enter)).await;

        // +2: echoed command + system alert response
        assert_eq!(app.state.blocks.len(), initial_blocks + 2);
        match &app.state.blocks[initial_blocks + 1] {
            UiBlock::SystemAlert { text, .. } => {
                assert!(text.contains("/clear"), "help text should mention /clear");
            }
            other => panic!("expected SystemAlert, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn clear_command_clears_blocks() {
        let (mut app, _) = make_app();
        assert!(!app.state.blocks.is_empty()); // startup message

        type_text(&mut app, "/clear");
        app.handle_key(press(KeyCode::Enter)).await;

        assert!(app.state.blocks.is_empty());
    }

    #[tokio::test]
    async fn model_command_shows_active_provider() {
        let (mut app, _) = make_app();
        let initial_blocks = app.state.blocks.len();

        type_text(&mut app, "/model");
        app.handle_key(press(KeyCode::Enter)).await;

        // +2: echoed command + system alert response
        assert_eq!(app.state.blocks.len(), initial_blocks + 2);
        match &app.state.blocks[initial_blocks + 1] {
            UiBlock::SystemAlert { text, .. } => {
                assert!(
                    text.contains("Active provider:"),
                    "model command should show active provider: {text}"
                );
            }
            other => panic!("expected SystemAlert, got {other:?}"),
        }
    }

    // ── Message submission ──────────────────────────────────────────────────

    #[tokio::test]
    async fn submit_message_sets_busy_and_adds_human_block() {
        let (mut app, client) = make_app();
        let initial_blocks = app.state.blocks.len();

        type_text(&mut app, "hello agent");
        app.handle_key(press(KeyCode::Enter)).await;

        assert!(app.state.is_busy);

        let human_block = &app.state.blocks[initial_blocks];
        match human_block {
            UiBlock::HumanMessage { text, .. } => {
                assert_eq!(text, "hello agent");
            }
            other => panic!("expected HumanMessage, got {other:?}"),
        }

        // Give the spawn a moment to execute
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let messages = client.submitted_messages.lock().await;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "hello agent");
    }

    #[tokio::test]
    async fn submit_message_error_sends_run_errored_event() {
        let (mut app, client) = make_app();
        client.fail_submit_run("connection refused").await;

        type_text(&mut app, "test");
        app.handle_key(press(KeyCode::Enter)).await;

        assert!(app.state.is_busy);

        // Poll the event channel with a timeout, looking for the RunErrored event
        // that the spawned submit task sends back on failure.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.state.is_busy {
            match tokio::time::timeout_at(deadline, app.events.recv()).await {
                Ok(Some(TuiEvent::Network(agent_event))) => {
                    app.state.apply_event(agent_event);
                }
                Ok(Some(_)) => {} // tick, connection lost, etc.
                Ok(None) | Err(_) => break,
            }
        }

        assert!(!app.state.is_busy, "is_busy should be false after error");

        let has_error_alert = app.state.blocks.iter().any(|b| match b {
            UiBlock::SystemAlert { text, .. } => text.contains("connection refused"),
            _ => false,
        });
        assert!(has_error_alert, "should show connection error in chat log");
    }

    // ── Network events ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn network_event_sets_connected_status() {
        let (mut app, _) = make_app();
        assert_eq!(app.state.connection_status, ConnectionStatus::Connecting);

        app.state.connection_status = ConnectionStatus::Connected;
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            provider: "mock".to_string(),
            max_iterations: 10,
        });

        assert_eq!(app.state.connection_status, ConnectionStatus::Connected);
        assert!(app.state.is_busy);
    }

    #[tokio::test]
    async fn run_errored_event_resets_busy_and_adds_alert() {
        let (mut app, _) = make_app();
        app.state.is_busy = true;
        let initial_blocks = app.state.blocks.len();

        app.state.apply_event(AgentEvent::RunErrored {
            run_id: "r1".to_string(),
            session_id: "s1".to_string(),
            error: "provider timeout".to_string(),
        });

        assert!(!app.state.is_busy);
        match &app.state.blocks[initial_blocks] {
            UiBlock::SystemAlert { text, .. } => {
                assert!(text.contains("provider timeout"));
            }
            other => panic!("expected SystemAlert, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn connection_lost_sets_disconnected() {
        let (mut app, _) = make_app();
        app.state.connection_status = ConnectionStatus::Connected;

        app.state.connection_status = ConnectionStatus::Disconnected;
        app.state.flash_error("Connection to daemon lost");

        assert_eq!(app.state.connection_status, ConnectionStatus::Disconnected);
        assert!(app.state.last_error.is_some());
        assert_eq!(
            app.state.last_error.as_ref().unwrap().message,
            "Connection to daemon lost"
        );
    }

    #[tokio::test]
    async fn connection_lost_while_busy_resets_busy() {
        let (mut app, _) = make_app();
        app.state.connection_status = ConnectionStatus::Connected;
        app.state.is_busy = true;

        // Simulate ConnectionLost handling from the run loop
        app.state.connection_status = ConnectionStatus::Disconnected;
        app.state.flash_error("Connection to daemon lost");
        if app.state.is_busy {
            app.state.is_busy = false;
            app.push_system_alert(
                "Connection lost while waiting for response. Restart the daemon and try again.",
            );
        }

        assert!(!app.state.is_busy);
        let has_disconnect_alert = app.state.blocks.iter().any(|b| match b {
            UiBlock::SystemAlert { text, .. } => text.contains("Connection lost"),
            _ => false,
        });
        assert!(has_disconnect_alert);
    }

    // ── Scrolling ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn scroll_accounts_for_wrapped_lines() {
        let (mut app, _) = make_app();
        app.state.blocks.clear();

        // Add a message that's longer than a typical viewport width
        let long_text = "a".repeat(200);
        app.state.blocks.push(UiBlock::HumanMessage {
            text: long_text,
            timestamp: chrono::Utc::now(),
        });

        // Render into a narrow terminal (40 cols wide) to force wrapping
        let backend = TestBackend::new(40, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut md = MarkdownRenderer::new();
        let theme = crate::theme::Theme::new();

        terminal
            .draw(|f| {
                crate::widgets::chat_log::render(f, f.area(), &mut app.state, &theme, &mut md);
            })
            .unwrap();

        // With 40-col viewport (38 inner after borders), "[HH:MM] You: " + 200 'a's
        // wraps to multiple visual lines. total_lines must be > 1.
        assert!(
            app.state.scroll.total_lines > 1,
            "total_lines should account for wrapping, got {}",
            app.state.scroll.total_lines
        );
    }

    #[tokio::test]
    async fn scroll_with_many_messages_allows_full_navigation() {
        let (mut app, _) = make_app();
        app.state.blocks.clear();

        for i in 0..50 {
            app.state.blocks.push(UiBlock::HumanMessage {
                text: format!("Message {i}: some content here"),
                timestamp: chrono::Utc::now(),
            });
        }

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut md = MarkdownRenderer::new();
        let theme = crate::theme::Theme::new();

        terminal
            .draw(|f| {
                crate::widgets::chat_log::render(f, f.area(), &mut app.state, &theme, &mut md);
            })
            .unwrap();

        assert!(app.state.scroll.auto_follow);
        assert_eq!(app.state.scroll.offset, 0);
        assert!(
            app.state.scroll.total_lines >= 50,
            "expected >= 50 total lines, got {}",
            app.state.scroll.total_lines
        );

        // Scroll all the way up
        let max = app.state.scroll.total_lines;
        app.state.scroll.scroll_up(max);
        assert!(!app.state.scroll.auto_follow);
        assert!(app.state.scroll.offset > 0);

        // Scroll back to bottom
        app.state.scroll.scroll_to_bottom();
        assert!(app.state.scroll.auto_follow);
        assert_eq!(app.state.scroll.offset, 0);
    }

    #[tokio::test]
    async fn page_down_scrolls_one_page_not_to_bottom() {
        let (mut app, _) = make_app();
        app.state.blocks.clear();

        for i in 0..100 {
            app.state.blocks.push(UiBlock::HumanMessage {
                text: format!("Message {i}"),
                timestamp: chrono::Utc::now(),
            });
        }

        // Simulate dimensions: 100 logical lines, 22-line viewport
        app.state.scroll.update_dimensions(100, 22);

        // Scroll all the way up
        app.state.scroll.scroll_up(100);
        let offset_after_up = app.state.scroll.offset;
        assert!(offset_after_up > 0);

        // PageDown = viewport_height - 1 = 21 lines down
        app.state.scroll.page_down();
        let offset_after_page_down = app.state.scroll.offset;
        assert!(
            offset_after_page_down < offset_after_up,
            "PageDown should reduce offset"
        );
        assert!(
            offset_after_page_down > 0,
            "PageDown should not jump to bottom from far up"
        );
        assert_eq!(offset_after_up - offset_after_page_down, 21);
    }

    // ── Full event flow ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn full_conversation_flow() {
        let (mut app, _) = make_app();
        let initial_blocks = app.state.blocks.len();

        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".to_string(),
            session_id: "test-session".to_string(),
            provider: "mock".to_string(),
            max_iterations: 10,
        });
        assert!(app.state.is_busy);

        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".to_string(),
            session_id: "test-session".to_string(),
            iteration: 0,
            delta: "Hello! ".to_string(),
        });
        assert_eq!(app.state.streaming_text, Some("Hello! ".to_string()));

        app.state.apply_event(AgentEvent::TextDelta {
            run_id: "r1".to_string(),
            session_id: "test-session".to_string(),
            iteration: 0,
            delta: "How can I help?".to_string(),
        });
        assert_eq!(
            app.state.streaming_text,
            Some("Hello! How can I help?".to_string())
        );

        app.state.apply_event(AgentEvent::RunFinished {
            run_id: "r1".to_string(),
            session_id: "test-session".to_string(),
            reason: RunStopReason::Completed,
            total_iterations: 1,
            final_answer: Some("Hello! How can I help?".to_string()),
            usage: None,
        });

        assert!(!app.state.is_busy);
        assert!(app.state.streaming_text.is_none());
        assert_eq!(app.state.blocks.len(), initial_blocks + 1);
        match &app.state.blocks[initial_blocks] {
            UiBlock::AssistantMessage { text, .. } => {
                assert_eq!(text, "Hello! How can I help?");
            }
            other => panic!("expected AssistantMessage, got {other:?}"),
        }
    }

    // ── Provider picker ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn provider_command_opens_picker() {
        let (mut app, _) = make_app();

        type_text(&mut app, "/provider");
        app.handle_key(press(KeyCode::Enter)).await;

        assert!(
            app.provider_picker.active,
            "provider picker should be active after /provider"
        );
        assert!(!app.provider_picker.loading, "loading should be resolved");
        assert!(
            !app.provider_picker.providers.is_empty(),
            "should have providers populated"
        );
        assert_eq!(app.provider_picker.current_provider, "mock");
    }

    #[tokio::test]
    async fn provider_picker_esc_dismisses() {
        let (mut app, _) = make_app();

        type_text(&mut app, "/provider");
        app.handle_key(press(KeyCode::Enter)).await;
        assert!(app.provider_picker.active);

        app.handle_key(press(KeyCode::Esc)).await;
        assert!(
            !app.provider_picker.active,
            "Esc should dismiss provider picker"
        );
    }

    #[tokio::test]
    async fn provider_picker_navigation() {
        let (mut app, _) = make_app();

        type_text(&mut app, "/provider");
        app.handle_key(press(KeyCode::Enter)).await;

        // Mock client returns ["mock", "anthropic", "openai"], current = "mock"
        // Pre-selected at index 0 ("mock")
        assert_eq!(app.provider_picker.selected, 0);

        // Navigate down
        app.handle_key(press(KeyCode::Down)).await;
        assert_eq!(app.provider_picker.selected, 1);

        app.handle_key(press(KeyCode::Down)).await;
        assert_eq!(app.provider_picker.selected, 2);

        // Wrap around
        app.handle_key(press(KeyCode::Down)).await;
        assert_eq!(app.provider_picker.selected, 0);

        // Navigate up wraps
        app.handle_key(press(KeyCode::Up)).await;
        assert_eq!(app.provider_picker.selected, 2);
    }

    #[tokio::test]
    async fn provider_picker_enter_switches_provider() {
        let (mut app, _) = make_app();
        let initial_blocks = app.state.blocks.len();

        type_text(&mut app, "/provider");
        app.handle_key(press(KeyCode::Enter)).await;

        // Navigate to "anthropic" (index 1)
        app.handle_key(press(KeyCode::Down)).await;
        assert_eq!(app.provider_picker.selected, 1);

        // Press Enter to select
        app.handle_key(press(KeyCode::Enter)).await;

        assert!(
            !app.provider_picker.active,
            "picker should be dismissed after selection"
        );

        // Should have a system alert confirming the switch
        // +2: echoed /provider command + switch confirmation
        let has_switch_alert = app.state.blocks.iter().any(|b| match b {
            UiBlock::SystemAlert { text, .. } => text.contains("Provider switched to"),
            _ => false,
        });
        assert!(
            has_switch_alert,
            "should show provider switch confirmation. blocks: {:?}",
            &app.state.blocks[initial_blocks..]
        );
    }

    #[tokio::test]
    async fn provider_with_arg_does_not_open_picker() {
        let (mut app, _) = make_app();

        type_text(&mut app, "/provider mock");
        app.handle_key(press(KeyCode::Enter)).await;

        assert!(
            !app.provider_picker.active,
            "/provider <name> should directly switch, not open picker"
        );
    }

    // ── Rendering smoke tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn app_renders_without_panic() {
        let (mut app, _) = make_app();

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = (0..24)
            .map(|y| {
                (0..80)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            content.contains("Session Log"),
            "rendered output should contain chat log title"
        );
    }

    #[tokio::test]
    async fn app_renders_with_messages_and_busy_indicator() {
        let (mut app, _) = make_app();

        app.state.blocks.push(UiBlock::HumanMessage {
            text: "Hello".to_string(),
            timestamp: chrono::Utc::now(),
        });
        app.state.apply_event(AgentEvent::RunStarted {
            run_id: "r1".to_string(),
            session_id: "test-session".to_string(),
            provider: "mock".to_string(),
            max_iterations: 10,
        });

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer().clone();
        let content: String = (0..24)
            .map(|y| {
                (0..80)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(content.contains("You:"), "should show user message");
        assert!(
            content.contains('\u{2026}'), // ellipsis from animated spinner verb
            "should show busy indicator: {content}"
        );
    }

    #[tokio::test]
    async fn app_renders_on_narrow_terminal() {
        let (mut app, _) = make_app();

        app.state.blocks.push(UiBlock::HumanMessage {
            text: "A longer message to test wrapping on narrow terminals".to_string(),
            timestamp: chrono::Utc::now(),
        });

        // Very narrow terminal — should not panic
        let backend = TestBackend::new(30, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::ui::draw(f, &mut app)).unwrap();
    }
}
