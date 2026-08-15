//! An mdBook preprocessor that highlights ` ```roo ` code fences using the
//! project's own tree-sitter-roo grammar and `highlights.scm` query,
//! instead of mdBook's default highlight.js (which has no notion of roo
//! at all).

use anyhow::{Context, Result};
use mdbook_preprocessor::book::{Book, BookItem};
use mdbook_preprocessor::errors::Error as MdbookError;
use mdbook_preprocessor::{parse_input, Preprocessor, PreprocessorContext};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter, HtmlRenderer};

/// The capture names this binary recognizes from `highlights.scm`. Each
/// becomes a `roo-<name-with-dots-as-dashes>` CSS class on the rendered
/// `<span>`. Keep in sync with `tree-sitter-roo/queries/highlights.scm`.
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "comment.documentation",
    "constant",
    "constructor",
    "function",
    "function.method",
    "keyword",
    "label",
    "number",
    "number.float",
    "operator",
    "property",
    "punctuation.bracket",
    "punctuation.delimiter",
    "string",
    "string.escape",
    "type",
    "type.builtin",
    "variable.builtin",
    "variable.parameter",
];

struct RooHighlight;

impl Preprocessor for RooHighlight {
    fn name(&self) -> &str {
        "roo-highlight"
    }

    fn run(&self, _ctx: &PreprocessorContext, mut book: Book) -> Result<Book, MdbookError> {
        let mut config = HighlightConfiguration::new(
            tree_sitter_roo::LANGUAGE.into(),
            "roo",
            tree_sitter_roo::HIGHLIGHTS_QUERY,
            tree_sitter_roo::INJECTIONS_QUERY,
            tree_sitter_roo::LOCALS_QUERY,
        )
        .map_err(|e| MdbookError::msg(format!("failed to build roo highlight query: {e}")))?;
        config.configure(HIGHLIGHT_NAMES);

        let mut highlighter = Highlighter::new();

        book.for_each_mut(|item| {
            if let BookItem::Chapter(chapter) = item {
                chapter.content = highlight_roo_fences(&chapter.content, &mut highlighter, &config);
            }
        });

        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> Result<bool, MdbookError> {
        Ok(renderer == "html")
    }
}

/// Walks `content` line by line, replacing every ` ```roo ` ... ` ``` `
/// fence with a pre-rendered, syntax-highlighted `<pre>` block. Everything
/// else passes through untouched.
fn highlight_roo_fences(content: &str, highlighter: &mut Highlighter, config: &HighlightConfiguration) -> String {
    let mut output = String::with_capacity(content.len());
    let mut lines = content.lines().peekable();

    while let Some(line) = lines.next() {
        if line.trim_start() == "```roo" {
            let mut code = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start() == "```" {
                    break;
                }
                code.push_str(inner);
                code.push('\n');
            }

            // Blank lines on both sides so pulldown-cmark reliably treats
            // this as a raw HTML block rather than trying to interpret it
            // as markdown/inline content.
            output.push('\n');
            output.push_str(&render_html(&code, highlighter, config));
            output.push_str("\n\n");
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }

    output
}

fn render_html(code: &str, highlighter: &mut Highlighter, config: &HighlightConfiguration) -> String {
    let events = match highlighter.highlight(config, code.as_bytes(), None, |_| None) {
        Ok(events) => events,
        Err(_) => return escape_as_plain(code),
    };

    let events: Vec<Result<HighlightEvent, tree_sitter_highlight::Error>> = events.collect();

    let mut renderer = HtmlRenderer::new();
    let render_result = renderer.render(events.into_iter(), code.as_bytes(), &|highlight: Highlight, output: &mut Vec<u8>| {
        output.extend_from_slice(b"class=\"roo-");
        output.extend_from_slice(HIGHLIGHT_NAMES[highlight.0].replace('.', "-").as_bytes());
        output.extend_from_slice(b"\"");
    });

    if render_result.is_err() {
        return escape_as_plain(code);
    }

    let mut html = String::from("<pre class=\"roo-highlight\"><code>");
    for line in renderer.lines() {
        html.push_str(line);
    }
    html.push_str("</code></pre>");
    html
}

fn escape_as_plain(code: &str) -> String {
    let escaped = code
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<pre class=\"roo-highlight\"><code>{escaped}</code></pre>")
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);

    if let Some(sub) = args.next() {
        if sub == "supports" {
            let renderer = args.next().unwrap_or_default();
            let supported = RooHighlight.supports_renderer(&renderer).unwrap_or(false);
            std::process::exit(if supported { 0 } else { 1 });
        }
    }

    let (ctx, book) = parse_input(std::io::stdin()).context("failed to parse mdBook preprocessor input")?;
    let processed_book = RooHighlight.run(&ctx, book).map_err(|e| anyhow::anyhow!(e))?;
    serde_json::to_writer(std::io::stdout(), &processed_book).context("failed to write processed book")?;

    Ok(())
}
