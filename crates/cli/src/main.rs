use std::env;
use std::fs;
use std::ops::Range;
use std::process::ExitCode;

use ariadne::{Color, Fmt, Label, Report, ReportKind, sources};
use ast::{Item, ItemKind, ModKind};
use chumsky::Parser;
use typecheck::{CheckedProgram, Diagnostic, Level, Locale};

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

    let mut cache = sources([(path.clone(), source.as_str())]);

    let tokens = match lexer::tokenize_all(&source) {
        Ok(tokens) => tokens,
        Err(err) => {
            eprintln!("error: failed to lex `{path}`: {err:?}");
            return ExitCode::FAILURE;
        }
    };

    let mut state = parser::State::default();
    let items = match parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
    {
        Ok(items) => items,
        Err(errors) => {
            for error in &errors {
                report_parse_error(&path, error).eprint(&mut cache).ok();
            }
            return ExitCode::FAILURE;
        }
    };
    let names = state.0.clone();

    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CheckedProgram::check(&items, state.0)
    }));
    let frozen = match checked {
        Ok(frozen) => frozen,
        Err(_) => {
            eprintln!(
                "\nerror: the type checker doesn't support a construct in `{path}` yet (see panic above) -- this is a known current limitation, not a bug in this file"
            );
            return ExitCode::FAILURE;
        }
    };

    println!("items in `{path}`:");
    for item in &items {
        print_item(&frozen, &names, item, 1);
    }

    let diagnostics = frozen.diagnostics(Locale::EnUs);
    println!();
    if diagnostics.is_empty() {
        println!("no diagnostics -- `{path}` type checks cleanly");
        ExitCode::SUCCESS
    } else {
        println!("{} diagnostic(s):\n", diagnostics.len());
        for diagnostic in &diagnostics {
            report_diagnostic(&path, diagnostic).eprint(&mut cache).ok();
        }
        ExitCode::FAILURE
    }
}

fn print_item(cx: &CheckedProgram, names: &parser::Interner, item: &Item, depth: usize) {
    let indent = "  ".repeat(depth);
    match &item.kind {
        ItemKind::Fn(f) => match cx.binding_at(f.ident.span.start) {
            Some(binding) => println!(
                "{indent}fn {}: {}",
                names.resolve(f.ident.symbol),
                cx.render_binding_type(binding)
            ),
            None => println!("{indent}fn {} (unresolved)", names.resolve(f.ident.symbol)),
        },
        ItemKind::TyAlias(alias) => match cx.binding_at(alias.ident.span.start) {
            Some(binding) => println!(
                "{indent}type {} = {}",
                names.resolve(alias.ident.symbol),
                cx.render_binding_type(binding)
            ),
            None => println!(
                "{indent}type {} (unresolved)",
                names.resolve(alias.ident.symbol)
            ),
        },
        ItemKind::Struct(ident, ..) => println!("{indent}struct {}", names.resolve(ident.symbol)),
        ItemKind::Enum(ident, ..) => println!("{indent}enum {}", names.resolve(ident.symbol)),
        ItemKind::Trait(t) => println!("{indent}trait {}", names.resolve(t.ident.symbol)),
        ItemKind::Mod(ident, ModKind::Loaded(items)) => {
            println!("{indent}mod {} {{", names.resolve(ident.symbol));
            for item in items {
                print_item(cx, names, item, depth + 1);
            }
            println!("{indent}}}");
        }
        ItemKind::Mod(ident, ModKind::Unloaded) => {
            println!("{indent}mod {}; (unloaded)", names.resolve(ident.symbol));
        }
        ItemKind::Use(_) => println!("{indent}use ..."),
        ItemKind::Impl(_) => println!("{indent}impl ..."),
    }
}

fn report_kind(level: Level) -> (ReportKind<'static>, Color) {
    match level {
        Level::Error => (ReportKind::Error, Color::Red),
        Level::Warning => (ReportKind::Warning, Color::Yellow),
        Level::Note => (
            ReportKind::Custom("note", Color::BrightBlue),
            Color::BrightBlue,
        ),
        Level::Help => (
            ReportKind::Custom("help", Color::BrightGreen),
            Color::BrightGreen,
        ),
    }
}

fn emphasize(message: &str, ranges: &[Range<usize>]) -> String {
    if ranges.is_empty() {
        return message.to_owned();
    }
    let mut sorted: Vec<Range<usize>> = ranges.to_vec();
    sorted.sort_by_key(|range| range.start);

    let mut out = String::with_capacity(message.len() + 16 * sorted.len());
    let mut cursor = 0;
    for range in sorted {
        if range.start < cursor || range.end > message.len() || range.start > range.end {
            continue;
        }
        out.push_str(&message[cursor..range.start]);
        out.push_str(&format!("{}", (&message[range.clone()]).fg(Color::Magenta)));
        cursor = range.end;
    }
    out.push_str(&message[cursor..]);
    out
}

fn report_diagnostic(
    path: &str,
    diagnostic: &Diagnostic,
) -> Report<'static, (SourceId, Range<usize>)> {
    let id = path.to_owned();
    let (kind, color) = report_kind(diagnostic.level());
    let primary = diagnostic.span();
    let primary_range = primary.start..primary.end;
    let message = emphasize(diagnostic.message(), diagnostic.emphasis());

    let mut builder = Report::build(kind, (id.clone(), primary_range.clone()))
        .with_message(message.clone())
        .with_label(
            Label::new((id.clone(), primary_range))
                .with_message(message)
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
