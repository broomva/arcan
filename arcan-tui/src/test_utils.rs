use ratatui::{Frame, Terminal, backend::TestBackend, layout::Rect};

/// Render a widget function into a string for snapshot testing.
///
/// Creates a `TestBackend` with the given dimensions, invokes `render_fn`
/// (which should call `f.render_widget(...)`) and returns the buffer contents
/// as a multi-line string. Trailing whitespace per row is trimmed.
pub fn render_to_string<F>(width: u16, height: u16, render_fn: F) -> String
where
    F: FnOnce(&mut Frame, Rect),
{
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();

    terminal
        .draw(|f| {
            let area = Rect::new(0, 0, width, height);
            render_fn(f, area);
        })
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let mut lines = Vec::new();
    for y in 0..height {
        let mut row = String::new();
        for x in 0..width {
            let cell = &buffer[(x, y)];
            row.push_str(cell.symbol());
        }
        lines.push(row.trim_end().to_string());
    }

    // Join and trim trailing empty lines
    let result = lines.join("\n");
    result.trim_end().to_string()
}
