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
use lsp_types::request::{HoverRequest, InlayHintRequest, Request as _};
use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticRelatedInformation, DiagnosticSeverity,
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InlayHint,
    InlayHintParams, Location, MarkupContent, MarkupKind, OneOf, Position,
    PublishDiagnosticsParams, Range, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};
use typecheck::{CheckedProgram, Diagnostic as RooDiagnostic, Level, Locale};

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

fn main_loop(connection: Connection) -> Result<(), Box<dyn Error + Sync + Send>> {
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
        InlayHintRequest::METHOD => {
            let params: InlayHintParams = serde_json::from_value(req.params)?;
            let text_document = params.text_document;
            let range = params.range;
            let inlay_hints = documents
                .get(&text_document.uri)
                .and_then(|text| inlay_hints(text, range));
            let response = Response::new_ok(req.id, inlay_hints);
            connection.sender.send(Message::Response(response))?;
        }
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
            let mut text = documents.remove(&uri).unwrap_or_default();
            for change in params.content_changes {
                match change.range {
                    Some(range) => {
                        let index = LineIndex::new(&text);
                        let start = index.offset(&text, range.start);
                        let end = index.offset(&text, range.end);
                        text.replace_range(start..end, &change.text);
                    }
                    None => text = change.text,
                }
            }
            publish(connection, &uri, &text)?;
            documents.insert(uri, text);
        }
        DidCloseTextDocument::METHOD => {
            let params: DidCloseTextDocumentParams = serde_json::from_value(not.params)?;
            documents.remove(&params.text_document.uri);
            send_diagnostics(connection, params.text_document.uri, Vec::new())?;
        }
        _ => {}
    }
    Ok(())
}

fn publish(
    connection: &Connection,
    uri: &Uri,
    text: &str,
) -> Result<(), Box<dyn Error + Sync + Send>> {
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

fn check_items(items: &[Box<ast::Item>], names: parser::Interner) -> Option<CheckedProgram> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CheckedProgram::check(items, names)
    }))
    .ok()
}

fn check(text: &str, uri: &Uri) -> Vec<LspDiagnostic> {
    let index = LineIndex::new(text);

    let tokens = match lexer::tokenize_all(text) {
        Ok(tokens) => tokens,
        Err(_) => return Vec::new(),
    };

    let mut state = parser::State::default();
    let items = match parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
    {
        Ok(items) => items,
        Err(errors) => {
            return errors
                .iter()
                .map(|error| parse_error_diagnostic(&index, text, error))
                .collect();
        }
    };

    let Some(cx) = check_items(&items, state.0) else {
        return Vec::new();
    };

    cx.diagnostics(Locale::EnUs)
        .iter()
        .map(|d| roo_diagnostic(&index, text, uri, d))
        .collect()
}

fn hover_at(text: &str, position: Position) -> Option<Hover> {
    let tokens = lexer::tokenize_all(text).ok()?;
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .ok()?;
    let cx = check_items(&items, state.0)?;

    let index = LineIndex::new(text);
    let offset = index.offset(text, position);

    let ty = match cx.def_at(offset) {
        Some(symbol) => cx.describe_def(symbol),
        None => cx.type_symbol_at(offset)?.to_owned(),
    };

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("```roo\n{ty}\n```"),
        }),
        range: None,
    })
}

fn inlay_hints(text: &str, range: Range) -> Option<Vec<InlayHint>> {
    let tokens = lexer::tokenize_all(text).ok()?;
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .ok()?;
    let cx = check_items(&items, state.0)?;
    Some(vec![])
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
