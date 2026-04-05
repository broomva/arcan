use ratatui::text::{Line, Span};
use ratskin::RatSkin;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

/// Renders markdown text into ratatui `Line`s using `ratskin` (termimad).
///
/// Supports tables, bold, italic, code blocks, lists, headings, blockquotes.
/// Results are cached by (content hash, width) to avoid re-parsing on every frame.
pub struct MarkdownRenderer {
    skin: RatSkin,
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
            skin: RatSkin::default(),
            cache: HashMap::new(),
            capacity: 16,
        }
    }

    /// Render markdown `input` into a Vec of ratatui Lines.
    ///
    /// `width` is the available terminal columns for line wrapping and table sizing.
    /// Results are cached by content hash + width.
    pub fn render(&mut self, input: &str, width: u16) -> Vec<Line<'static>> {
        let hash = Self::hash_key(input, width);

        if let Some(cached) = self.cache.get(&hash) {
            return cached.clone();
        }

        let parsed = RatSkin::parse_text(input);
        let lines: Vec<Line<'_>> = self.skin.parse(parsed, width);
        // Convert to owned for caching.
        let lines: Vec<Line<'static>> = lines.into_iter().map(line_to_owned).collect();

        if self.cache.len() >= self.capacity {
            self.cache.clear();
        }
        self.cache.insert(hash, lines.clone());

        lines
    }

    /// Check whether the input looks like it contains markdown formatting.
    pub fn has_markdown(input: &str) -> bool {
        input.contains("```")
            || input.contains("**")
            || input.contains("## ")
            || input.contains("# ")
            || input.contains("- ")
            || input.contains("1. ")
            || input.contains('`')
            || input.contains("> ")
            || input.contains("*")
            || input.contains("| ")
    }

    fn hash_key(s: &str, width: u16) -> u64 {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        width.hash(&mut hasher);
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
        let lines = r.render("Hello, world!", 80);
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
        let lines = r.render("This is **bold** text.", 80);
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
        let lines = r.render(input, 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(
            text.contains("fn main()"),
            "code block should contain the code content"
        );
    }

    #[test]
    fn table_renders_multiple_lines() {
        let mut r = MarkdownRenderer::new();
        let input = "| Name | Value |\n|------|-------|\n| foo  | 1     |\n| bar  | 2     |";
        let lines = r.render(input, 80);
        assert!(
            lines.len() >= 3,
            "table should produce at least 3 lines (header + separator + 2 rows), got {}",
            lines.len()
        );
    }

    #[test]
    fn cache_returns_same_result() {
        let mut r = MarkdownRenderer::new();
        let input = "# Heading\nSome text.";
        let first = r.render(input, 80);
        let second = r.render(input, 80);
        assert_eq!(first.len(), second.len());
    }

    #[test]
    fn cache_evicts_on_overflow() {
        let mut r = MarkdownRenderer::new();
        r.capacity = 2;
        r.render("text 1", 80);
        r.render("text 2", 80);
        assert_eq!(r.cache.len(), 2);
        r.render("text 3", 80);
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
        assert!(MarkdownRenderer::has_markdown("| col1 | col2 |"));
    }

    #[test]
    fn has_markdown_rejects_plain() {
        assert!(!MarkdownRenderer::has_markdown("Hello world"));
        assert!(!MarkdownRenderer::has_markdown("No formatting here."));
    }

    #[test]
    fn inline_code_renders() {
        let mut r = MarkdownRenderer::new();
        let lines = r.render("Use `cargo test` to run tests.", 80);
        let text: String = lines
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert!(text.contains("cargo test"));
    }

    #[test]
    fn list_renders() {
        let mut r = MarkdownRenderer::new();
        let lines = r.render("- item one\n- item two\n- item three", 80);
        assert!(lines.len() >= 3);
    }
}
