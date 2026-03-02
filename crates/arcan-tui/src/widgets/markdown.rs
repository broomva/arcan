use ratatui::text::{Line, Span, Text};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Renders markdown text into ratatui `Text` with syntax highlighting.
///
/// Wraps `tui_markdown` with an LRU-style cache (keyed by text hash) to avoid
/// re-parsing unchanged messages on every frame.
pub struct MarkdownRenderer {
    cache: HashMap<u64, Vec<Line<'static>>>,
    /// Maximum number of cached entries before eviction.
    capacity: usize,
}

impl Default for MarkdownRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownRenderer {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            capacity: 16,
        }
    }

    /// Render markdown `input` into a Vec of ratatui Lines.
    ///
    /// Results are cached by content hash. If the cache is full, it is cleared
    /// (simple eviction — chat messages are append-only so old entries are
    /// unlikely to be needed again).
    pub fn render(&mut self, input: &str) -> Vec<Line<'static>> {
        let hash = Self::hash_str(input);

        if let Some(cached) = self.cache.get(&hash) {
            return cached.clone();
        }

        let text: Text<'_> = tui_markdown::from_str(input);
        // Convert borrowed lines to owned so they can be cached.
        let lines: Vec<Line<'static>> = text.lines.into_iter().map(line_to_owned).collect();

        if self.cache.len() >= self.capacity {
            self.cache.clear();
        }
        self.cache.insert(hash, lines.clone());

        lines
    }

    /// Check whether the input looks like it contains markdown formatting.
    /// Used to decide whether to use the markdown renderer or plain text.
    pub fn has_markdown(input: &str) -> bool {
        // Quick heuristic: check for common markdown patterns
        input.contains("```")
            || input.contains("**")
            || input.contains("## ")
            || input.contains("# ")
            || input.contains("- ")
            || input.contains("1. ")
            || input.contains('`')
            || input.contains("> ")
            || input.contains("*")
    }

    fn hash_str(s: &str) -> u64 {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }
}

/// Convert a potentially-borrowed Line into a fully owned Line<'static>.
fn line_to_owned(line: Line<'_>) -> Line<'static> {
    let spans: Vec<Span<'static>> = line
        .spans
        .into_iter()
        .map(|span| Span::styled(span.content.into_owned(), span.style))
        .collect();
    Line::from(spans).style(line.style)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_renders() {
        let mut r = MarkdownRenderer::new();
        let lines = r.render("Hello, world!");
        assert!(!lines.is_empty());
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("Hello, world!"));
    }

    #[test]
    fn bold_renders_styled() {
        let mut r = MarkdownRenderer::new();
        let lines = r.render("This is **bold** text.");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("bold"));
    }

    #[test]
    fn code_block_renders() {
        let mut r = MarkdownRenderer::new();
        let input = "```rust\nfn main() {}\n```";
        let lines = r.render(input);
        assert!(lines.len() >= 3, "code block should produce multiple lines");
    }

    #[test]
    fn cache_returns_same_result() {
        let mut r = MarkdownRenderer::new();
        let input = "# Heading\nSome text.";
        let first = r.render(input);
        let second = r.render(input);
        assert_eq!(first.len(), second.len());
    }

    #[test]
    fn cache_evicts_on_overflow() {
        let mut r = MarkdownRenderer::new();
        r.capacity = 2;
        r.render("text 1");
        r.render("text 2");
        assert_eq!(r.cache.len(), 2);
        r.render("text 3");
        // Cache was cleared and now only has the new entry
        assert_eq!(r.cache.len(), 1);
    }

    #[test]
    fn has_markdown_detects_patterns() {
        assert!(MarkdownRenderer::has_markdown("```rust\ncode\n```"));
        assert!(MarkdownRenderer::has_markdown("**bold**"));
        assert!(MarkdownRenderer::has_markdown("## Heading"));
        assert!(MarkdownRenderer::has_markdown("- list item"));
        assert!(MarkdownRenderer::has_markdown("`inline code`"));
        assert!(MarkdownRenderer::has_markdown("> blockquote"));
    }

    #[test]
    fn has_markdown_rejects_plain() {
        assert!(!MarkdownRenderer::has_markdown("Hello world"));
        assert!(!MarkdownRenderer::has_markdown("No formatting here."));
    }

    #[test]
    fn inline_code_renders() {
        let mut r = MarkdownRenderer::new();
        let lines = r.render("Use `cargo test` to run tests.");
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("cargo test"));
    }

    #[test]
    fn list_renders() {
        let mut r = MarkdownRenderer::new();
        let lines = r.render("- item one\n- item two\n- item three");
        // Each list item should produce a separate line
        assert!(lines.len() >= 3);
    }
}
