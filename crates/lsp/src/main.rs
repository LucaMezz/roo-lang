//! roo-lang's language server.
//!
//! On every `didOpen`/`didChange`, reruns the same `lex -> parse ->
//! resolve -> lower_signatures -> check` pipeline the reference CLI
//! does (see `crates/cli/src/main.rs`) against the whole document, and
//! reports whatever the checker found via
//! `textDocument/publishDiagnostics`. Also answers `textDocument/hover`
//! by re-running the same pipeline and reporting whatever symbol's name
//! is written at the cursor's position, via
//! [`typecheck::TypeCheckContext::symbol_at`]. No go-to-definition or
//! completion yet.

// `HashMap<Uri, _>` trips clippy's `mutable_key_type` lint: `Uri`
// wraps `fluent_uri`'s internal lazily-computed parse-position cache (a
// `Cell`), which clippy can't prove doesn't affect hashing/equality --
// but `Uri`'s actual `Hash`/`Eq` impls are derived from its string
// content alone, so that cache is irrelevant to correctness here.
#![allow(clippy::mutable_key_type)]

mod line_index;

use std::collections::HashMap;
use std::error::Error;

use chumsky::Parser;
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{HoverRequest, Request as _};
use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticRelatedInformation, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, Location, MarkupContent,
    MarkupKind, OneOf, Position, PublishDiagnosticsParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use typecheck::{Diagnostic as RooDiagnostic, Level, TypeCheckContext};

use crate::line_index::LineIndex;

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(false)),
        ..Default::default()
    };
    let server_capabilities = serde_json::to_value(capabilities)?;
    let initialize_params = connection.initialize(server_capabilities)?;
    let _params: InitializeParams = serde_json::from_value(initialize_params)?;

    main_loop(connection)?;
    io_threads.join()?;
    Ok(())
}

/// Takes `connection` by value rather than by reference so it (and its
/// `sender`, specifically) is dropped the moment this function returns
/// -- `io_threads.join()` back in `main` waits for the writer thread to
/// see its channel disconnect before it can finish, which only happens
/// once every `Sender<Message>` handle, including this one, is gone.
/// Keeping `connection` alive past this point (e.g. by holding only a
/// `&Connection` here and letting `main` drop the owned value later)
/// deadlocks the join: the writer thread blocks forever waiting for a
/// message that will never come from a channel nothing has closed.
fn main_loop(connection: Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
    // Every open document's current full text, keyed by URI -- kept
    // around (unlike diagnostics, which are recomputed and thrown away
    // per publish) so a `hover` *request*, which arrives independently
    // of any `didChange` *notification*, always has the text to check
    // against.
    let mut documents: HashMap<Uri, String> = HashMap::new();

    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(&connection, req, &documents)?;
            }
            Message::Response(_) => {}
            Message::Notification(not) => handle_notification(&connection, not, &mut documents)?,
        }
    }
    Ok(())
}

fn handle_request(
    connection: &Connection,
    req: Request,
    documents: &HashMap<Uri, String>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match req.method.as_str() {
        HoverRequest::METHOD => {
            let params: HoverParams = serde_json::from_value(req.params)?;
            let text_document = params.text_document_position_params.text_document;
            let position = params.text_document_position_params.position;
            let hover = documents
                .get(&text_document.uri)
                .and_then(|text| hover_at(text, position));
            let response = Response::new_ok(req.id, hover);
            connection.sender.send(Message::Response(response))?;
        }
        // Nothing else is handled yet -- `shutdown` is already dealt
        // with by `handle_shutdown` before this is reached, so any
        // other request just gets a "not implemented" error back
        // rather than being silently dropped, which would otherwise
        // hang a client waiting on a response that will never come.
        _ => {
            let response = Response::new_err(
                req.id,
                lsp_server::ErrorCode::MethodNotFound as i32,
                format!("`{}` is not implemented yet", req.method),
            );
            connection.sender.send(Message::Response(response))?;
        }
    }
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    not: Notification,
    documents: &mut HashMap<Uri, String>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = params.text_document.uri;
            let text = params.text_document.text;
            publish(connection, &uri, &text)?;
            documents.insert(uri, text);
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
            let uri = params.text_document.uri;
            // Full sync (see `TextDocumentSyncKind::FULL` above): the
            // last change event carries the entire new document text,
            // so only it matters.
            if let Some(change) = params.content_changes.into_iter().next_back() {
                publish(connection, &uri, &change.text)?;
                documents.insert(uri, change.text);
            }
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            documents.remove(&params.text_document.uri);
            // Clear diagnostics for a closed file rather than leaving
            // stale ones in the editor's problem list.
            send_diagnostics(connection, params.text_document.uri, Vec::new())?;
        }
        _ => {}
    }
    Ok(())
}

fn publish(connection: &Connection, uri: &Uri, text: &str) -> Result<(), Box<dyn Error + Sync + Send>> {
    let diagnostics = check(text, uri);
    send_diagnostics(connection, uri.clone(), diagnostics)
}

fn send_diagnostics(
    connection: &Connection,
    uri: Uri,
    diagnostics: Vec<LspDiagnostic>,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let params = PublishDiagnosticsParams {
        uri,
        diagnostics,
        version: None,
    };
    let notification = Notification::new(PublishDiagnostics::METHOD.to_owned(), params);
    connection
        .sender
        .send(Message::Notification(notification))?;
    Ok(())
}

/// Runs `resolve -> lower_signatures -> check` against already-parsed
/// `items`, catching the checker's known-limitation panics the same
/// way the CLI does (it's still under active development and reaches a
/// real `unimplemented!()` on some constructs -- an expected, known-
/// limitation outcome right now, not something to propagate as a
/// crash). `None` if checking hit one of those, since there's nothing
/// meaningfully checked to hand back in that case.
fn check_items(items: &[Box<ast::Item>]) -> Option<TypeCheckContext> {
    let mut cx = TypeCheckContext::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.resolve(items);
        cx.lower_signatures(items);
        cx.check(items);
    }));
    result.ok().map(|()| cx)
}

/// Runs the same pipeline the CLI does and converts whatever it finds
/// into LSP diagnostics. A lex failure has no span to report against
/// (same limitation the CLI has) and is dropped; a parse failure is
/// reported the same way the CLI's own `report_parse_error` does,
/// since checking can't proceed past a file that doesn't even parse.
fn check(text: &str, uri: &Uri) -> Vec<LspDiagnostic> {
    let index = LineIndex::new(text);

    let tokens = match lexer::tokenize_all(text) {
        Ok(tokens) => tokens,
        Err(_) => return Vec::new(),
    };

    let items = match parser::module().parse(parser::input(tokens)).into_result() {
        Ok(items) => items,
        Err(errors) => {
            return errors
                .iter()
                .map(|error| parse_error_diagnostic(&index, text, error))
                .collect();
        }
    };

    let Some(cx) = check_items(&items) else {
        return Vec::new();
    };

    cx.diagnostics()
        .iter()
        .map(|d| roo_diagnostic(&index, text, uri, d))
        .collect()
}

/// Lexes, parses, and checks `text`, then reports the type of whichever
/// symbol's name `position` lands on, if any -- `None` covers both "the
/// file doesn't even parse" and "there's no symbol written here"
/// (whitespace, punctuation, a literal, ...) alike, since a client
/// treats a missing hover result the same way in either case.
fn hover_at(text: &str, position: Position) -> Option<Hover> {
    let tokens = lexer::tokenize_all(text).ok()?;
    let items = parser::module().parse(parser::input(tokens)).into_result().ok()?;
    let mut cx = check_items(&items)?;

    let index = LineIndex::new(text);
    let offset = index.offset(text, position);
    let symbol = cx.symbol_at(offset)?;
    let ty = cx.render_symbol_type(symbol);

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```roo\n{ty}\n```"),
        }),
        range: None,
    })
}

fn severity(level: Level) -> DiagnosticSeverity {
    match level {
        Level::Error => DiagnosticSeverity::ERROR,
        Level::Warning => DiagnosticSeverity::WARNING,
        Level::Note => DiagnosticSeverity::INFORMATION,
        Level::Help => DiagnosticSeverity::HINT,
    }
}

fn roo_diagnostic(
    index: &LineIndex,
    text: &str,
    uri: &Uri,
    diagnostic: &RooDiagnostic,
) -> LspDiagnostic {
    let related_information = if diagnostic.related().is_empty() {
        None
    } else {
        Some(
            diagnostic
                .related()
                .iter()
                .map(|(span, message)| DiagnosticRelatedInformation {
                    location: Location {
                        uri: uri.clone(),
                        range: index.range(text, *span),
                    },
                    message: message.clone(),
                })
                .collect(),
        )
    };

    // LSP diagnostics have no room for `Diagnostic::emphasis`'s
    // in-message highlighting -- there's no equivalent concept in the
    // protocol, so it's dropped here; the plain message text is all a
    // client can show either way.
    let mut message = diagnostic.message().to_owned();
    for note in diagnostic.notes() {
        message.push('\n');
        message.push_str(note);
    }

    LspDiagnostic {
        range: index.range(text, diagnostic.span()),
        severity: Some(severity(diagnostic.level())),
        source: Some("roo".to_owned()),
        message,
        related_information,
        ..Default::default()
    }
}

fn parse_error_diagnostic(
    index: &LineIndex,
    text: &str,
    error: &chumsky::error::Simple<'_, lexer::Token<'_>>,
) -> LspDiagnostic {
    let span = error.span();
    let range = index.range(
        text,
        ast::Span {
            start: span.start,
            end: span.end,
        },
    );
    let message = match error.found() {
        Some(token) => format!("unexpected token `{token:?}`"),
        None => "unexpected end of input".to_owned(),
    };
    LspDiagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("roo".to_owned()),
        message,
        ..Default::default()
    }
}
