# Arcan Shell TUI Input — Design Spec

## Goal

Replace the current `stdin.read_line()` REPL with a proper terminal UI input system matching Noesis/Claude Code UX.

## Requirements

### Input Area (always visible at bottom)
- Fixed input line at the bottom of the terminal (like Noesis's `>` prompt)
- User can type at any time — during streaming, during thinking, while idle
- Input is always visible (never hidden or suppressed)
- Multi-line input support (Enter sends, Shift+Enter for newlines if applicable)

### Message Queuing
- Messages typed during agent thinking/streaming are queued
- Queued messages shown stacked above the input line (dim/grey)
- "Press up to edit queued messages" hint when queue is non-empty
- Queue processed sequentially after current turn completes

### Status Bar
- Line above the input showing: provider, model, context %, tokens, cost, permissions mode
- Updates after each turn

### Output Area (scrolling above)
- Streaming text renders above the status bar
- Spinner/thinking indicator in the output area (not blocking input)
- Markdown rendering (current `StreamingMarkdown` renderer)

### Keyboard Shortcuts
- **Enter**: Send current input (or next queued message if input empty)
- **Escape**: Exit shell gracefully (clean shutdown, persist state)
- **Up/Down**: Navigate queued messages for editing
- **Ctrl+C**: Cancel current agent run (if running), otherwise exit
- **/**: Show command hints (existing feature)

### Architecture
- `crossterm` raw mode (already a dependency)
- Dedicated render thread for output area
- Input handled on main thread via `crossterm::event::read()`
- Agent loop runs in a separate thread, communicates via channels
- Writer thread (already exists) handles persistence

### Layout
```
┌─────────────────────────────────────────────────┐
│ [streaming output / agent response]              │
│ ...                                              │
│ ◉ Done (2.1s · ↓ 114 tokens · $0.0594)         │
│                                                  │
│ what else?          ← queued message (dim)       │
│ aha?                ← queued message (dim)       │
├─────────────────────────────────────────────────┤
│ Provider: haiku | 200K | 8% | $0.12 | bypass    │
├─────────────────────────────────────────────────┤
│ > type here...                                   │
└─────────────────────────────────────────────────┘
```

## Reference Implementation
- Noesis: `apps/cli/src/` — React/Ink-based TUI
- Key patterns: `ConsoleOAuthFlow.tsx`, message input component
- Arcan already has `arcan-tui` crate (ratatui-based) for the daemon TUI client

## Non-Goals (for first iteration)
- Tab completion for file paths
- Syntax highlighting in input
- Mouse support
- Split pane resizing
