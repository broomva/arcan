use crate::models::ui_block::ToolStatus;
use crate::theme::Theme;
use ratatui::text::{Line, Span};

/// Render tool execution details as a sequence of Lines for inline display
/// in the chat log. Shows tool name, status, arguments summary, and result
/// preview.
pub fn render_tool_lines(
    tool_name: &str,
    status: &ToolStatus,
    arguments: &serde_json::Value,
    result: &Option<serde_json::Value>,
    timestamp: &chrono::DateTime<chrono::Utc>,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let ts = timestamp.format("%H:%M").to_string();

    // Header line: [HH:MM] Tool tool_name [STATUS]
    let (status_str, status_style) = match status {
        ToolStatus::Running => ("Running...", theme.tool_label),
        ToolStatus::Success => ("OK", theme.tool_success),
        ToolStatus::Error(_) => ("ERR", theme.tool_error),
    };

    lines.push(Line::from(vec![
        Span::styled(format!("[{ts}] "), theme.timestamp),
        Span::styled("Tool ", theme.tool_label),
        Span::styled(format!("{tool_name} "), theme.tool_label),
        Span::styled(format!("[{status_str}]"), status_style),
    ]));

    // Arguments summary (compact single-line for small args, multi-line for large)
    let args_str = format_json_compact(arguments);
    if !args_str.is_empty() && args_str != "null" && args_str != "{}" {
        let truncated = truncate_str(&args_str, 120);
        lines.push(Line::from(vec![
            Span::styled("       args: ", theme.timestamp),
            Span::raw(truncated),
        ]));
    }

    // Result preview
    if let Some(res) = result {
        let res_str = format_json_compact(res);
        if !res_str.is_empty() && res_str != "null" {
            let truncated = truncate_str(&res_str, 120);
            let style = match status {
                ToolStatus::Success => theme.tool_success,
                _ => theme.timestamp,
            };
            lines.push(Line::from(vec![
                Span::styled("       result: ", style),
                Span::raw(truncated),
            ]));
        }
    }

    // Error detail
    if let ToolStatus::Error(err) = status {
        let truncated = truncate_str(err, 120);
        lines.push(Line::from(vec![
            Span::styled("       error: ", theme.tool_error),
            Span::styled(truncated, theme.tool_error),
        ]));
    }

    lines
}

/// Format a JSON value as a compact single-line string.
/// Objects are shown as `{key: value, ...}`, strings without quotes for readability.
fn format_json_compact(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        _ => serde_json::to_string(value).unwrap_or_default(),
    }
}

/// Truncate a string to `max_len` characters, appending "..." if truncated.
fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .nth(max_len.saturating_sub(3))
            .map(|(i, _)| i)
            .unwrap_or(max_len.saturating_sub(3));
        format!("{}...", &s[..boundary])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn renders_success_tool() {
        let theme = Theme::new();
        let lines = render_tool_lines(
            "shell",
            &ToolStatus::Success,
            &serde_json::json!({"command": "ls -la"}),
            &Some(serde_json::json!("file1.rs\nfile2.rs")),
            &Utc::now(),
            &theme,
        );
        assert!(lines.len() >= 2, "should have header + args + result");
        let header: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("shell"));
        assert!(header.contains("[OK]"));
    }

    #[test]
    fn renders_error_tool() {
        let theme = Theme::new();
        let lines = render_tool_lines(
            "read_file",
            &ToolStatus::Error("file not found".to_string()),
            &serde_json::json!({"path": "/tmp/missing"}),
            &None,
            &Utc::now(),
            &theme,
        );
        let all_text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(all_text.contains("[ERR]"));
        assert!(all_text.contains("file not found"));
    }

    #[test]
    fn renders_running_tool() {
        let theme = Theme::new();
        let lines = render_tool_lines(
            "search",
            &ToolStatus::Running,
            &serde_json::json!({}),
            &None,
            &Utc::now(),
            &theme,
        );
        let header: String = lines[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(header.contains("[Running...]"));
    }

    #[test]
    fn skips_empty_args() {
        let theme = Theme::new();
        let lines = render_tool_lines(
            "noop",
            &ToolStatus::Success,
            &serde_json::json!({}),
            &None,
            &Utc::now(),
            &theme,
        );
        // Only header line, no args or result lines
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn truncates_long_strings() {
        let result = truncate_str(&"x".repeat(200), 50);
        assert!(result.len() <= 53); // 47 chars + "..."
        assert!(result.ends_with("..."));
    }

    #[test]
    fn format_json_string_unquoted() {
        let val = serde_json::json!("hello world");
        assert_eq!(format_json_compact(&val), "hello world");
    }

    #[test]
    fn format_json_object() {
        let val = serde_json::json!({"key": "value"});
        let result = format_json_compact(&val);
        assert!(result.contains("key"));
        assert!(result.contains("value"));
    }
}
