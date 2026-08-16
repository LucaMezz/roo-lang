//! roo-lang's language server.
//!
//! The first slice of an LSP for roo: on every `didOpen`/`didChange`,
//! reruns the same `lex -> parse -> resolve -> lower_signatures ->
//! check` pipeline the reference CLI does (see `crates/cli/src/main.rs`)
//! against the whole document, and reports whatever the checker found
//! via `textDocument/publishDiagnostics`. No hover, go-to-definition,
//! or completion yet -- just enough to see the same diagnostics the CLI
//! reports directly inline in the editor.

mod line_index;

use std::error::Error;

use chumsky::Parser;
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticRelatedInformation, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, Location, OneOf, PublishDiagnosticsParams, ServerCapabilities,
    TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use typecheck::{Diagnostic as RooDiagnostic, Level, TypeCheckContext};

use crate::line_index::LineIndex;

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, io_threads) = Connection::stdio();

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        hover_provider: None,
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
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                handle_request(&connection, req)?;
            }
            Message::Response(_) => {}
            Message::Notification(not) => handle_notification(&connection, not)?,
        }
    }
    Ok(())
}

/// Nothing but `shutdown` is handled yet -- `handle_shutdown` above
/// already deals with that before this is reached, so any other
/// request just gets a "not implemented" error back rather than being
/// silently dropped, which would otherwise hang a client waiting on a
/// response that will never come.
fn handle_request(
    connection: &Connection,
    req: Request,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let response = Response::new_err(
        req.id,
        lsp_server::ErrorCode::MethodNotFound as i32,
        format!("`{}` is not implemented yet", req.method),
    );
    connection.sender.send(Message::Response(response))?;
    Ok(())
}

fn handle_notification(
    connection: &Connection,
    not: Notification,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    match not.method.as_str() {
        DidOpenTextDocument::METHOD => {
            let params: DidOpenTextDocumentParams = serde_json::from_value(not.params)?;
            publish(
                connection,
                params.text_document.uri,
                &params.text_document.text,
            )?;
        }
        DidChangeTextDocument::METHOD => {
            let params: DidChangeTextDocumentParams = serde_json::from_value(not.params)?;
            // Full sync (see `TextDocumentSyncKind::FULL` above): the
            // last change event carries the entire new document text,
            // so only it matters.
            if let Some(change) = params.content_changes.into_iter().next_back() {
                publish(connection, params.text_document.uri, &change.text)?;
            }
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            // Clear diagnostics for a closed file rather than leaving
            // stale ones in the editor's problem list.
            send_diagnostics(connection, params.text_document.uri, Vec::new())?;
        }
        _ => {}
    }
    Ok(())
}

fn publish(
    connection: &Connection,
    uri: Uri,
    text: &str,
) -> Result<(), Box<dyn Error + Sync + Send>> {
    let diagnostics = check(text, &uri);
    send_diagnostics(connection, uri, diagnostics)
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

    let mut cx = TypeCheckContext::new();
    // The checker is still under active development and reaches a real
    // `unimplemented!()` on some constructs -- see the CLI's own
    // handling of this for why that's an expected, known-limitation
    // outcome right now rather than something to propagate as a crash.
    let checked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.resolve(&items);
        cx.lower_signatures(&items);
        cx.check(&items);
    }));
    if checked.is_err() {
        return Vec::new();
    }

    cx.diagnostics()
        .iter()
        .map(|d| roo_diagnostic(&index, text, uri, d))
        .collect()
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
