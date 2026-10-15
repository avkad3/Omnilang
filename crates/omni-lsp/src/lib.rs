//! The OmniLang language server (`omni lsp`).
//!
//! Speaks LSP over stdio using `lsp-server` (the framework behind
//! rust-analyzer). Features: diagnostics in the user's language, completion
//! that turns romanized or English input into native keywords and names,
//! hover with every language's spelling, formatting, and the `omni.view`
//! command that returns a document translated into another language.

pub mod analysis;
pub mod position;

use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidChangeConfiguration, DidChangeTextDocument, DidCloseTextDocument, DidOpenTextDocument,
    Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Completion, ExecuteCommand, Formatting, HoverRequest, Request as _};
use lsp_types::{
    CompletionList, CompletionOptions, CompletionParams, CompletionResponse, DocumentFormattingParams,
    ExecuteCommandOptions, ExecuteCommandParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, MarkupContent, MarkupKind, OneOf, PublishDiagnosticsParams, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use omni_core::diag::MESSAGE_LANGS;
use omni_core::glossary::Glossary;
use omni_core::pack::Packs;
use serde::Deserialize;
use serde_json::Value;

use analysis::Doc;

pub const VIEW_COMMAND: &str = "omni.view";

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

/// User settings (VS Code `omni.*`, or LSP initialization options).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Language for diagnostics: en, hi, ta.
    pub message_language: String,
    /// Language assumed for a new, keyword-less document.
    pub typing_language: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            message_language: "en".into(),
            typing_language: "en".into(),
        }
    }
}

impl Settings {
    fn sanitized(mut self) -> Self {
        if !MESSAGE_LANGS.contains(&self.message_language.as_str()) {
            self.message_language = "en".into();
        }
        if !MESSAGE_LANGS.contains(&self.typing_language.as_str()) {
            self.typing_language = "en".into();
        }
        self
    }
}

pub struct Server {
    packs: Packs,
    docs: HashMap<String, String>,
    settings: Settings,
}

/// Runs the server on stdin/stdout until the client shuts it down.
pub fn run_stdio() -> Result<()> {
    let (connection, io_threads) = Connection::stdio();
    serve(connection)?;
    io_threads.join()?;
    Ok(())
}

pub fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions::default()),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        execute_command_provider: Some(ExecuteCommandOptions {
            commands: vec![VIEW_COMMAND.into()],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Runs the server on an established connection (also used by tests).
pub fn serve(connection: Connection) -> Result<()> {
    let caps = serde_json::to_value(capabilities())?;
    let init = serde_json::json!({
        "capabilities": caps,
        "serverInfo": ServerInfo { name: "omni".into(), version: Some(env!("CARGO_PKG_VERSION").into()) },
    });
    let (id, params) = connection.initialize_start()?;
    connection.initialize_finish(id, init)?;
    let params: InitializeParams = serde_json::from_value(params)?;
    let settings = params
        .initialization_options
        .and_then(|v| serde_json::from_value::<Settings>(v).ok())
        .unwrap_or_default()
        .sanitized();

    let mut server = Server {
        packs: Packs::builtin(),
        docs: HashMap::new(),
        settings,
    };
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if connection.handle_shutdown(&req)? {
                    return Ok(());
                }
                let resp = server.request(req);
                connection.sender.send(Message::Response(resp))?;
            }
            Message::Notification(note) => {
                for out in server.notification(note) {
                    connection.sender.send(Message::Notification(out))?;
                }
            }
            Message::Response(_) => {}
        }
    }
    Ok(())
}

/// Filesystem path of a `file:` URI (percent-decoded; `/c:/...` → `c:/...`).
fn uri_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                decoded.push(b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let mut path = String::from_utf8(decoded).ok()?;
    if path.len() > 2 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b':' {
        path.remove(0); // Windows drive letter
    }
    Some(PathBuf::from(path))
}

/// The project glossary next to the document, if any.
fn glossary_for(uri: &str) -> Glossary {
    uri_path(uri)
        .and_then(|p| p.parent().map(|d| d.join("omni.glossary.toml")))
        .and_then(|g| std::fs::read_to_string(g).ok())
        .and_then(|text| Glossary::from_toml(&text).ok())
        .unwrap_or_default()
}

impl Server {
    fn with_doc<T>(&self, uri: &Uri, f: impl FnOnce(&Doc) -> T) -> Option<T> {
        let text = self.docs.get(uri.as_str())?;
        let glossary = glossary_for(uri.as_str());
        let doc = Doc {
            text,
            packs: &self.packs,
            glossary: &glossary,
        };
        Some(f(&doc))
    }

    fn publish(&self, uri: Uri) -> Option<Notification> {
        let lang = self.settings.message_language.clone();
        let diagnostics = self
            .with_doc(&uri, |d| analysis::diagnostics(d, &lang))
            .unwrap_or_default();
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics,
            version: None,
        };
        Some(Notification::new(PublishDiagnostics::METHOD.into(), params))
    }

    fn notification(&mut self, note: Notification) -> Vec<Notification> {
        match note.method.as_str() {
            DidOpenTextDocument::METHOD => {
                let Ok(p) = serde_json::from_value::<lsp_types::DidOpenTextDocumentParams>(note.params)
                else {
                    return vec![];
                };
                self.docs
                    .insert(p.text_document.uri.as_str().to_string(), p.text_document.text);
                self.publish(p.text_document.uri).into_iter().collect()
            }
            DidChangeTextDocument::METHOD => {
                let Ok(p) = serde_json::from_value::<lsp_types::DidChangeTextDocumentParams>(note.params)
                else {
                    return vec![];
                };
                if let Some(change) = p.content_changes.into_iter().last() {
                    self.docs
                        .insert(p.text_document.uri.as_str().to_string(), change.text);
                }
                self.publish(p.text_document.uri).into_iter().collect()
            }
            DidCloseTextDocument::METHOD => {
                let Ok(p) = serde_json::from_value::<lsp_types::DidCloseTextDocumentParams>(note.params)
                else {
                    return vec![];
                };
                self.docs.remove(p.text_document.uri.as_str());
                let params = PublishDiagnosticsParams {
                    uri: p.text_document.uri,
                    diagnostics: vec![],
                    version: None,
                };
                vec![Notification::new(PublishDiagnostics::METHOD.into(), params)]
            }
            DidChangeConfiguration::METHOD => {
                if let Some(settings) = note.params.get("settings").and_then(|s| s.get("omni")) {
                    if let Ok(s) = serde_json::from_value::<Settings>(settings.clone()) {
                        self.settings = s.sanitized();
                    }
                }
                // Re-publish so diagnostics switch language immediately.
                let uris: Vec<String> = self.docs.keys().cloned().collect();
                uris.into_iter()
                    .filter_map(|u| u.parse::<Uri>().ok())
                    .filter_map(|u| self.publish(u))
                    .collect()
            }
            _ => vec![],
        }
    }

    fn request(&mut self, req: Request) -> Response {
        let id = req.id.clone();
        let result = match req.method.as_str() {
            Completion::METHOD => self.completion(req.params),
            HoverRequest::METHOD => self.hover(req.params),
            Formatting::METHOD => self.formatting(req.params),
            ExecuteCommand::METHOD => self.execute(req.params),
            _ => {
                return error(
                    id,
                    ErrorCode::MethodNotFound,
                    format!("unsupported method {}", req.method),
                )
            }
        };
        match result {
            Ok(value) => Response::new_ok(id, value),
            Err(e) => error(id, ErrorCode::InvalidParams, e.to_string()),
        }
    }

    fn completion(&self, params: Value) -> Result<Value> {
        let p: CompletionParams = serde_json::from_value(params)?;
        let doc = p.text_document_position;
        let typing = self.settings.typing_language.clone();
        let items = self
            .with_doc(&doc.text_document.uri, |d| {
                analysis::completion(d, doc.position, &typing)
            })
            .unwrap_or_default();
        // Incomplete: the client asks again as the word grows, which is what
        // makes typing-as-transliteration work.
        Ok(serde_json::to_value(CompletionResponse::List(CompletionList {
            is_incomplete: true,
            items,
        }))?)
    }

    fn hover(&self, params: Value) -> Result<Value> {
        let p: HoverParams = serde_json::from_value(params)?;
        let pos = p.text_document_position_params;
        let text = self
            .with_doc(&pos.text_document.uri, |d| analysis::hover(d, pos.position))
            .flatten();
        Ok(serde_json::to_value(text.map(|value| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            }),
            range: None,
        }))?)
    }

    fn formatting(&self, params: Value) -> Result<Value> {
        let p: DocumentFormattingParams = serde_json::from_value(params)?;
        let typing = self.settings.typing_language.clone();
        let edits = self
            .with_doc(&p.text_document.uri, |d| analysis::format(d, &typing))
            .flatten();
        Ok(serde_json::to_value(edits)?)
    }

    /// `omni.view [uri, lang]` → the document rendered in `lang`.
    fn execute(&self, params: Value) -> Result<Value> {
        let p: ExecuteCommandParams = serde_json::from_value(params)?;
        if p.command != VIEW_COMMAND {
            return Err(format!("unknown command {}", p.command).into());
        }
        let (Some(uri), Some(lang)) = (
            p.arguments.first().and_then(Value::as_str),
            p.arguments.get(1).and_then(Value::as_str),
        ) else {
            return Err("omni.view expects [uri, language]".into());
        };
        let uri: Uri = uri.parse()?;
        let text = self.with_doc(&uri, |d| analysis::view(d, lang)).flatten();
        Ok(serde_json::to_value(text)?)
    }
}

fn error(id: RequestId, code: ErrorCode, message: String) -> Response {
    Response::new_err(id, code as i32, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_become_paths() {
        assert_eq!(
            uri_path("file:///c%3A/Users/x/a.om"),
            Some(PathBuf::from("c:/Users/x/a.om"))
        );
        assert_eq!(
            uri_path("file:///home/x/my%20prog.om"),
            Some(PathBuf::from("/home/x/my prog.om"))
        );
        assert_eq!(uri_path("untitled:Untitled-1"), None);
    }
}
