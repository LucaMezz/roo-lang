//! roo-lang's reference CLI.
//!
//! Lexes, parses, and type checks a single `.roo` file, then reports
//! what the checker found: every top-level item's resolved type, and
//! any diagnostics it collected along the way, rendered as rustc-style
//! source snippets via [`ariadne`]. Type checking is as far as the
//! compiler goes right now -- there's no interpreter or codegen yet --
//! so this is a debugging/inspection tool for the compiler itself, not
//! a way to actually run a roo program.

use std::env;
use std::fs;
use std::ops::Range;
use std::process::ExitCode;

use ariadne::{Color, Label, Report, ReportKind, sources};
use ast::{Ident, Item, ItemKind, ModKind, Path, PathSegment};
use chumsky::Parser;
use typecheck::{Diagnostic, Level, Namespace, TypeCheckContext};

/// The source id [`ariadne`] reports are built against -- just the file
/// path, since this CLI only ever looks at one file at a time.
type SourceId = String;

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: roo <file.roo>");
        return ExitCode::FAILURE;
    };

    let source = match fs::read_to_string(&path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("error: couldn't read `{path}`: {err}");
            return ExitCode::FAILURE;
        }
    };

    // Every diagnostic reported past this point renders against this one
    // file, so a single cache built once up front is enough.
    let mut cache = sources([(path.clone(), source.as_str())]);

    let tokens = match lexer::tokenize_all(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            // `LexError` carries no span -- lexing bails out at the first
            // unrecognized byte with no position attached, so there's no
            // snippet to render here. A real fix belongs in `lexer`, not
            // this CLI.
            eprintln!("error: failed to lex `{path}`: {err:?}");
            return ExitCode::FAILURE;
        }
    };

    let items = match parser::module().parse(parser::input(tokens)).into_result() {
        Ok(items) => items,
        Err(errors) => {
            for error in &errors {
                report_parse_error(&path, error).eprint(&mut cache).ok();
            }
            return ExitCode::FAILURE;
        }
    };

    let mut cx = TypeCheckContext::new();

    // The checker is still under active development and reaches a real
    // `unimplemented!()` on some constructs -- catch that rather than
    // taking the whole CLI down with an unexplained panic, since that's
    // an expected, known-limitation outcome at this stage, not a bug in
    // the input file.
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.resolve(&items);
        cx.lower_signatures(&items);
        cx.check(&items);
    }));
    if checked.is_err() {
        eprintln!(
            "\nerror: the type checker doesn't support a construct in `{path}` yet (see panic above) -- this is a known current limitation, not a bug in this file"
        );
        return ExitCode::FAILURE;
    }

    println!("items in `{path}`:");
    for item in &items {
        print_item(&mut cx, item, 1);
    }

    let diagnostics = cx.diagnostics();
    println!();
    if diagnostics.is_empty() {
        println!("no diagnostics -- `{path}` type checks cleanly");
        ExitCode::SUCCESS
    } else {
        println!("{} diagnostic(s):\n", diagnostics.len());
        for diagnostic in diagnostics {
            report_diagnostic(&path, diagnostic).eprint(&mut cache).ok();
        }
        ExitCode::FAILURE
    }
}

/// Prints one item's kind, name, and (for the item kinds the checker
/// actually resolves a type for) its checked type -- recursing into
/// loaded inline `mod { ... }` bodies at one extra level of indent.
fn print_item(cx: &mut TypeCheckContext, item: &Item, depth: usize) {
    let indent = "  ".repeat(depth);
    match &item.kind {
        ItemKind::Fn(f) => match symbol_type(cx, &f.ident, Namespace::Value) {
            Some(ty) => println!("{indent}fn {}: {ty}", f.ident.name),
            None => println!("{indent}fn {} (unresolved)", f.ident.name),
        },
        ItemKind::TyAlias(alias) => match symbol_type(cx, &alias.ident, Namespace::Type) {
            Some(ty) => println!("{indent}type {} = {ty}", alias.ident.name),
            None => println!("{indent}type {} (unresolved)", alias.ident.name),
        },
        ItemKind::Struct(ident, ..) => println!("{indent}struct {}", ident.name),
        ItemKind::Enum(ident, ..) => println!("{indent}enum {}", ident.name),
        ItemKind::Trait(t) => println!("{indent}trait {}", t.ident.name),
        ItemKind::Mod(ident, ModKind::Loaded(items)) => {
            println!("{indent}mod {} {{", ident.name);
            for item in items {
                print_item(cx, item, depth + 1);
            }
            println!("{indent}}}");
        }
        ItemKind::Mod(ident, ModKind::Unloaded) => {
            println!("{indent}mod {}; (unloaded)", ident.name);
        }
        ItemKind::Use(_) => println!("{indent}use ..."),
        ItemKind::Impl(_) => println!("{indent}impl ..."),
    }
}

/// Looks up the symbol a top-level item's name resolved to, and renders
/// its checked type -- `None` if the item never got a symbol at all
/// (e.g. it was declared in a scope this lookup can't see).
fn symbol_type(cx: &mut TypeCheckContext, ident: &Ident, ns: Namespace) -> Option<String> {
    let path = Path {
        segments: vec![PathSegment {
            ident: ident.clone(),
            args: None,
        }],
        span: ident.span,
    };
    let symbol = cx.resolve_path(&path, ns)?;
    Some(cx.render_symbol_type(symbol))
}

/// The color/label ariadne renders a diagnostic's severity with --
/// [`Level::Note`]/[`Level::Help`] aren't native [`ReportKind`]
/// variants, so they're given their own custom, rustc-styled kind
/// rather than being folded into [`ReportKind::Advice`] and losing
/// their distinct wording.
fn report_kind(level: Level) -> (ReportKind<'static>, Color) {
    match level {
        Level::Error => (ReportKind::Error, Color::Red),
        Level::Warning => (ReportKind::Warning, Color::Yellow),
        Level::Note => (ReportKind::Custom("note", Color::BrightBlue), Color::BrightBlue),
        Level::Help => (ReportKind::Custom("help", Color::BrightGreen), Color::BrightGreen),
    }
}

/// Converts one [`typecheck::Diagnostic`] into an [`ariadne::Report`]:
/// the primary span underlined in the severity's color, every related
/// span as its own captioned secondary label, and every note attached
/// via `with_note`.
///
/// The primary label repeats the report's own message as its caption
/// -- not just for symmetry with the related labels below, but
/// because ariadne only draws the underline/caret row under a label
/// that has a message attached; a label with only `.with_color(...)`
/// and no caption renders as inline-colored text with no underline at
/// all (confirmed directly against ariadne 0.6.0's rendering, not
/// documented behavior to take on faith). Without this, only
/// diagnostics that happen to carry a related span would ever show
/// rustc-style underlines.
fn report_diagnostic(path: &str, diagnostic: &Diagnostic) -> Report<'static, (SourceId, Range<usize>)> {
    let id = path.to_owned();
    let (kind, color) = report_kind(diagnostic.level());
    let primary = diagnostic.span();
    let primary_range = primary.start..primary.end;

    let mut builder = Report::build(kind, (id.clone(), primary_range.clone()))
        .with_message(diagnostic.message())
        .with_label(
            Label::new((id.clone(), primary_range))
                .with_message(diagnostic.message())
                .with_color(color),
        );

    for (span, message) in diagnostic.related() {
        builder = builder.with_label(
            Label::new((id.clone(), span.start..span.end))
                .with_message(message)
                .with_color(Color::Cyan),
        );
    }
    for note in diagnostic.notes() {
        builder = builder.with_note(note);
    }

    builder.finish()
}

/// Converts one chumsky parse error into an [`ariadne::Report`] the same
/// way [`report_diagnostic`] does for a checker diagnostic, so a parse
/// failure and a type error look like the same kind of thing to the
/// person reading them.
fn report_parse_error<'src>(
    path: &str,
    error: &chumsky::error::Simple<'src, lexer::Token<'src>>,
) -> Report<'static, (SourceId, Range<usize>)> {
    let id = path.to_owned();
    let span = error.span();
    let range = span.start..span.end;
    let message = match error.found() {
        Some(token) => format!("unexpected token `{token:?}`"),
        None => "unexpected end of input".to_owned(),
    };

    Report::build(ReportKind::Error, (id.clone(), range.clone()))
        .with_message(message.clone())
        .with_label(
            Label::new((id, range))
                .with_message(message)
                .with_color(Color::Red),
        )
        .finish()
}
