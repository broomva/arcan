//! Streaming terminal markdown renderer for arcan shell.
//!
//! Buffers incoming text tokens and renders completed lines with ANSI styling.
//! Handles headers, bold, italic, inline code, code blocks, bullets, and rules.

use std::io::Write;

/// ANSI escape codes for markdown styling.
mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const ITALIC: &str = "\x1b[3m";
    pub const CYAN: &str = "\x1b[36m";
    pub const YELLOW: &str = "\x1b[33m";
}

/// Streaming markdown renderer that buffers tokens and renders line-by-line.
pub struct StreamingMarkdown {
    buffer: String,
    in_code_block: bool,
    code_lang: String,
}

impl StreamingMarkdown {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            in_code_block: false,
            code_lang: String::new(),
        }
    }

    /// Feed a text delta. Renders completed lines to stdout immediately.
    pub fn push(&mut self, text: &str) {
        self.buffer.push_str(text);

        // Process all complete lines in the buffer.
        while let Some(newline_pos) = self.buffer.find('\n') {
            let line = self.buffer[..newline_pos].to_string();
            self.buffer = self.buffer[newline_pos + 1..].to_string();
            self.render_line(&line);
        }
    }

    /// Flush any remaining buffered text (partial line at end of response).
    pub fn flush(&mut self) {
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            self.render_line(&remaining);
        }
        if self.in_code_block {
            // Unterminated code block — reset style.
            let mut out = std::io::stdout().lock();
            let _ = write!(out, "{}", ansi::RESET);
            let _ = out.flush();
            self.in_code_block = false;
        }
    }

    fn render_line(&mut self, line: &str) {
        let mut out = std::io::stdout().lock();

        // Handle code block fences.
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if self.in_code_block {
                // Closing fence.
                let _ = writeln!(out, "{}", ansi::RESET);
                self.in_code_block = false;
                self.code_lang.clear();
            } else {
                // Opening fence.
                self.in_code_block = true;
                self.code_lang = trimmed.trim_start_matches('`').to_string();
                let label = if self.code_lang.is_empty() {
                    String::new()
                } else {
                    format!(" {}{}{}", ansi::DIM, self.code_lang, ansi::RESET)
                };
                let _ = writeln!(out, "{}{}", ansi::DIM, label);
            }
            let _ = out.flush();
            return;
        }

        // Inside code block — print with dim styling, no further parsing.
        if self.in_code_block {
            let _ = writeln!(out, "{}  {}", ansi::DIM, line);
            let _ = out.flush();
            return;
        }

        // Headers.
        if let Some(rest) = trimmed.strip_prefix("### ") {
            let _ = writeln!(out, "{}{}  {}{}", ansi::BOLD, ansi::CYAN, rest, ansi::RESET);
            let _ = out.flush();
            return;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            let _ = writeln!(out, "{}{} {}{}", ansi::BOLD, ansi::CYAN, rest, ansi::RESET);
            let _ = out.flush();
            return;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let _ = writeln!(out, "{}{}{}{}", ansi::BOLD, ansi::CYAN, rest, ansi::RESET);
            let _ = out.flush();
            return;
        }

        // Horizontal rules.
        if trimmed == "---" || trimmed == "***" || trimmed == "___" {
            let width = crossterm::terminal::size()
                .map(|(w, _)| w as usize)
                .unwrap_or(80);
            let rule: String = "\u{2500}".repeat(width.min(72));
            let _ = writeln!(out, "{}{}{}", ansi::DIM, rule, ansi::RESET);
            let _ = out.flush();
            return;
        }

        // Bullet points: `- item` or `* item` (but not `**bold**`).
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let styled = render_inline(rest);
            let _ = writeln!(out, "  \u{2022} {styled}");
            let _ = out.flush();
            return;
        }
        if trimmed.starts_with("* ") && !trimmed.starts_with("**") {
            let rest = &trimmed[2..];
            let styled = render_inline(rest);
            let _ = writeln!(out, "  \u{2022} {styled}");
            let _ = out.flush();
            return;
        }

        // Numbered lists: `1. item`.
        if trimmed.len() > 2 {
            if let Some(dot_pos) = trimmed.find(". ") {
                if dot_pos <= 3 && trimmed[..dot_pos].chars().all(|c| c.is_ascii_digit()) {
                    let num = &trimmed[..dot_pos];
                    let rest = &trimmed[dot_pos + 2..];
                    let styled = render_inline(rest);
                    let _ = writeln!(out, "  {num}. {styled}");
                    let _ = out.flush();
                    return;
                }
            }
        }

        // Blockquotes.
        if let Some(rest) = trimmed.strip_prefix("> ") {
            let styled = render_inline(rest);
            let _ = writeln!(out, "  {}\u{2502}{} {styled}", ansi::DIM, ansi::RESET);
            let _ = out.flush();
            return;
        }

        // Regular line with inline formatting.
        let styled = render_inline(line);
        let _ = writeln!(out, "{styled}");
        let _ = out.flush();
    }
}

/// Render inline markdown: **bold**, *italic*, `code`.
fn render_inline(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 32);
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Bold: **text**
        if i + 1 < len && chars[i] == '*' && chars[i + 1] == '*' {
            if let Some(end) = find_closing(&chars, i + 2, &['*', '*']) {
                result.push_str(ansi::BOLD);
                let inner: String = chars[i + 2..end].iter().collect();
                result.push_str(&inner);
                result.push_str(ansi::RESET);
                i = end + 2;
                continue;
            }
        }

        // Italic: *text* (single asterisk, not at word boundary issues)
        if chars[i] == '*' && (i + 1 < len && chars[i + 1] != '*' && chars[i + 1] != ' ') {
            if let Some(end) = find_single_closing(&chars, i + 1, '*') {
                result.push_str(ansi::ITALIC);
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&inner);
                result.push_str(ansi::RESET);
                i = end + 1;
                continue;
            }
        }

        // Inline code: `text`
        if chars[i] == '`' {
            if let Some(end) = find_single_closing(&chars, i + 1, '`') {
                result.push_str(ansi::YELLOW);
                let inner: String = chars[i + 1..end].iter().collect();
                result.push_str(&inner);
                result.push_str(ansi::RESET);
                i = end + 1;
                continue;
            }
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

/// Find closing double-char delimiter (e.g., `**`).
fn find_closing(chars: &[char], start: usize, delim: &[char; 2]) -> Option<usize> {
    let mut i = start;
    while i + 1 < chars.len() {
        if chars[i] == delim[0] && chars[i + 1] == delim[1] {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find closing single-char delimiter (e.g., `*` or `` ` ``).
fn find_single_closing(chars: &[char], start: usize, delim: char) -> Option<usize> {
    for i in start..chars.len() {
        if chars[i] == delim {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_bold() {
        let result = render_inline("hello **world** foo");
        assert!(result.contains(ansi::BOLD));
        assert!(result.contains("world"));
        assert!(result.contains(ansi::RESET));
    }

    #[test]
    fn inline_italic() {
        let result = render_inline("hello *world* foo");
        assert!(result.contains(ansi::ITALIC));
        assert!(result.contains("world"));
    }

    #[test]
    fn inline_code() {
        let result = render_inline("use `cargo test` here");
        assert!(result.contains(ansi::YELLOW));
        assert!(result.contains("cargo test"));
    }

    #[test]
    fn unclosed_bold_passes_through() {
        let result = render_inline("hello **world");
        assert_eq!(result, "hello **world");
    }

    #[test]
    fn code_block_state() {
        let mut md = StreamingMarkdown::new();
        assert!(!md.in_code_block);
        md.render_line("```rust");
        assert!(md.in_code_block);
        md.render_line("let x = 1;");
        assert!(md.in_code_block);
        md.render_line("```");
        assert!(!md.in_code_block);
    }

    #[test]
    fn streaming_line_detection() {
        let mut md = StreamingMarkdown::new();
        // Partial tokens that form a complete line.
        md.push("hel");
        assert!(!md.buffer.is_empty());
        md.push("lo\n");
        assert!(md.buffer.is_empty()); // line was flushed
    }
}
