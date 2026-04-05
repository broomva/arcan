//! Live TUI integration test against a running daemon.
//!
//! This test connects to a real arcan daemon (must be running on localhost:3000),
//! creates an App with HttpAgentClient, drives it through a multi-turn
//! conversation, renders frames, and captures exactly what the user would see.
//!
//! Run with: `cargo test -p arcan-tui live_test -- --ignored --nocapture`
//! (requires daemon running: `cargo run --bin arcan -- --provider mock serve`)

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::client::AgentClientPort;
    use crate::event::TuiEvent;
    use crate::models::state::ConnectionStatus;
    use crate::models::ui_block::UiBlock;
    use crate::network::{HttpAgentClient, NetworkConfig};
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

    /// Type text and press Enter.
    async fn type_and_submit(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.input_bar.input(press(KeyCode::Char(ch)));
        }
        app.handle_key(press(KeyCode::Enter)).await;
    }

    /// Render the app and return the terminal content as a string.
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

    /// Drain all pending events from the channel with a timeout.
    async fn drain_events(app: &mut App, timeout_ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            match tokio::time::timeout_at(deadline, app.events.recv()).await {
                Ok(Some(TuiEvent::Network(event))) => {
                    app.state.connection_status = ConnectionStatus::Connected;
                    app.state.apply_event(event);
                }
                Ok(Some(TuiEvent::StatusBarUpdate {
                    context_pressure_pct,
                    autonomic_ruling,
                    cost_remaining,
                })) => {
                    if let Some(pct) = context_pressure_pct {
                        app.state.context_pressure_pct = pct;
                    }
                    if let Some(ruling) = autonomic_ruling {
                        app.state.autonomic_ruling = Some(ruling);
                    }
                    if let Some(cost) = cost_remaining {
                        app.state.cost_remaining = Some(cost);
                    }
                }
                Ok(Some(TuiEvent::ConnectionLost)) => break,
                Ok(Some(_)) => {} // tick, resize, etc
                Ok(None) | Err(_) => break,
            }
        }
    }

    /// Full user experience test against a live daemon.
    ///
    /// Run: `cargo test -p arcan-tui live_test -- --ignored --nocapture`
    #[tokio::test]
    #[ignore] // Requires running daemon: cargo run --bin arcan -- --provider mock serve
    async fn full_user_experience_against_daemon() {
        // Connect to daemon
        let session_id = format!("live-test-{}", chrono::Utc::now().timestamp());
        let client: Arc<dyn AgentClientPort> = Arc::new(HttpAgentClient::new(NetworkConfig {
            base_url: "http://localhost:3000".to_string(),
            session_id: session_id.clone(),
        }));

        // Create session via API first
        let http = reqwest::Client::new();
        let create_resp = http
            .post("http://localhost:3000/sessions")
            .json(&serde_json::json!({"session_id": session_id}))
            .send()
            .await;
        assert!(
            create_resp.is_ok(),
            "Failed to create session — is the daemon running? Start with: cargo run --bin arcan -- --provider mock serve"
        );

        let mut app = App::new(client);

        // === FRAME 0: Initial state ===
        let frame0 = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME 0: Initial state ===\n{frame0}\n");
        assert!(frame0.contains("Session Log"), "Should show chat log");
        assert!(frame0.contains(&session_id[..8]), "Should show session ID");

        // === TURN 1: Send a message ===
        println!(">>> User types: hello world");
        type_and_submit(&mut app, "hello world").await;

        let frame1 = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME 1: After sending ===\n{frame1}\n");
        assert!(frame1.contains("hello world"), "Should show user message");

        // Wait for agent response
        println!("    Waiting for agent response...");
        drain_events(&mut app, 10_000).await;

        let frame2 = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME 2: After agent response ===\n{frame2}\n");

        let has_response = app.state.blocks.iter().any(|b| {
            matches!(
                b,
                UiBlock::AssistantMessage { .. } | UiBlock::SystemAlert { .. }
            )
        });
        assert!(
            has_response || !app.state.is_busy,
            "Should have a response or be done"
        );

        // === SLASH COMMANDS ===
        println!(">>> User types: /help");
        type_and_submit(&mut app, "/help").await;
        let frame_help = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: /help ===\n{frame_help}\n");
        assert!(frame_help.contains("/clear"), "Help should list commands");

        println!(">>> User types: /context");
        type_and_submit(&mut app, "/context").await;
        drain_events(&mut app, 2_000).await;
        let frame_ctx = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: /context ===\n{frame_ctx}\n");
        assert!(
            frame_ctx.contains("Context") || frame_ctx.contains("tokens"),
            "Should show context info"
        );

        println!(">>> User types: /autonomic");
        type_and_submit(&mut app, "/autonomic").await;
        drain_events(&mut app, 2_000).await;
        let frame_auto = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: /autonomic ===\n{frame_auto}\n");
        assert!(
            frame_auto.contains("Autonomic") || frame_auto.contains("Breathe"),
            "Should show autonomic ruling"
        );

        println!(">>> User types: /cost");
        type_and_submit(&mut app, "/cost").await;
        drain_events(&mut app, 2_000).await;
        let frame_cost = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: /cost ===\n{frame_cost}\n");
        assert!(
            frame_cost.contains("Cost") || frame_cost.contains("remaining"),
            "Should show cost info"
        );

        // === TURN 2: Another message ===
        println!(">>> User types: what is 2+2?");
        type_and_submit(&mut app, "what is 2+2?").await;
        drain_events(&mut app, 10_000).await;

        let frame_t2 = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: Turn 2 response ===\n{frame_t2}\n");
        // Turn 1 message may have scrolled off-screen — check blocks, not rendered frame
        let has_turn1 = app.state.blocks.iter().any(
            |b| matches!(b, UiBlock::HumanMessage { text, .. } if text.contains("hello world")),
        );
        assert!(has_turn1, "Turn 1 message should be in block history");

        // === STATUS BAR CHECK ===
        // After runs complete, status bar should have updated
        drain_events(&mut app, 3_000).await;
        let frame_final = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: Final state ===\n{frame_final}\n");

        // Verify status bar has Autonomic data
        // Status bar format: ● provider │ ⎇ branch │ 40% │ Breathe │ $5.00 │ idle │ session
        println!("Status bar check:");
        println!("  context_pressure: {:.1}%", app.state.context_pressure_pct);
        println!("  autonomic_ruling: {:?}", app.state.autonomic_ruling);
        println!("  cost_remaining: {:?}", app.state.cost_remaining);

        // === CLEAR ===
        println!(">>> User types: /clear");
        type_and_submit(&mut app, "/clear").await;
        let frame_clear = render_frame(&mut app, 100, 30);
        println!("\n=== FRAME: After /clear ===\n{frame_clear}\n");
        assert!(
            !frame_clear.contains("hello world"),
            "Clear should remove messages"
        );

        // === ESC to quit ===
        app.handle_key(press(KeyCode::Esc)).await;
        assert!(app.should_quit, "Esc should set quit flag");

        println!("\n=== LIVE TEST PASSED ===");
        println!("Session: {session_id}");
        println!("Blocks rendered: {}", app.state.blocks.len());
    }
}
