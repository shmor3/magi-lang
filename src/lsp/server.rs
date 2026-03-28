//! JSON-RPC 2.0 server over stdio — replaces tower-lsp.
//!
//! Reads LSP messages from stdin, dispatches to handlers, writes responses to stdout.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::{Arc, Mutex};

use crate::util::{JsonValue, OrderedMap, json_parse_value, json_to_string, json_int};

/// A JSON-RPC request/notification.
#[derive(Debug)]
pub struct RpcMessage {
    pub id: Option<JsonValue>,   // None for notifications
    pub method: String,
    pub params: JsonValue,
}

/// Client handle for sending notifications back to the editor.
#[derive(Clone)]
pub struct Client {
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Client {
    pub fn new(writer: Box<dyn Write + Send>) -> Self {
        Client { writer: Arc::new(Mutex::new(writer)) }
    }

    /// Send a notification (no id, no response expected).
    pub fn send_notification(&self, method: &str, params: JsonValue) {
        let msg = JsonValue::Object(OrderedMap::from([
            ("jsonrpc".into(), JsonValue::String("2.0".into())),
            ("method".into(), JsonValue::String(method.into())),
            ("params".into(), params),
        ]));
        let body = json_to_string(&msg);
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(header.as_bytes());
            let _ = w.write_all(body.as_bytes());
            let _ = w.flush();
        }
    }

    pub fn publish_diagnostics(&self, uri: &str, diagnostics: &[JsonValue], _version: Option<i32>) {
        let params = JsonValue::Object(OrderedMap::from([
            ("uri".into(), JsonValue::String(uri.into())),
            ("diagnostics".into(), JsonValue::Array(diagnostics.to_vec())),
        ]));
        self.send_notification("textDocument/publishDiagnostics", params);
    }

    pub fn log_message(&self, _type_: u32, message: &str) {
        let params = JsonValue::Object(OrderedMap::from([
            ("type".into(), json_int(_type_ as i64)),
            ("message".into(), JsonValue::String(message.into())),
        ]));
        self.send_notification("window/logMessage", params);
    }
}

/// Read one LSP message from a BufReader.
fn read_message(reader: &mut BufReader<impl Read>) -> Option<String> {
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => return None, // EOF
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() { break; } // End of headers
                if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
                    content_length = len_str.trim().parse().unwrap_or(0);
                }
            }
            Err(_) => return None,
        }
    }
    if content_length == 0 { return None; }

    let mut body = vec![0u8; content_length];
    match reader.read_exact(&mut body) {
        Ok(_) => String::from_utf8(body).ok(),
        Err(_) => None,
    }
}

/// Parse a JSON-RPC message from a JSON string.
fn parse_rpc(json_str: &str) -> Option<RpcMessage> {
    let val = json_parse_value(json_str).ok()?;
    let obj = match &val { JsonValue::Object(o) => o, _ => return None };
    let method = obj.get("method")?.as_str()?.to_string();
    let id = obj.get("id").cloned();
    let params = obj.get("params").cloned().unwrap_or(JsonValue::Null);
    Some(RpcMessage { id, method, params })
}

/// Send a JSON-RPC response.
fn send_response(writer: &Mutex<Box<dyn Write + Send>>, id: JsonValue, result: JsonValue) {
    let msg = JsonValue::Object(OrderedMap::from([
        ("jsonrpc".into(), JsonValue::String("2.0".into())),
        ("id".into(), id),
        ("result".into(), result),
    ]));
    let body = json_to_string(&msg);
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    if let Ok(mut w) = writer.lock() {
        let _ = w.write_all(header.as_bytes());
        let _ = w.write_all(body.as_bytes());
        let _ = w.flush();
    }
}

fn send_error(writer: &Mutex<Box<dyn Write + Send>>, id: JsonValue, code: i64, message: &str) {
    let msg = JsonValue::Object(OrderedMap::from([
        ("jsonrpc".into(), JsonValue::String("2.0".into())),
        ("id".into(), id),
        ("error".into(), JsonValue::Object(OrderedMap::from([
            ("code".into(), json_int(code)),
            ("message".into(), JsonValue::String(message.into())),
        ]))),
    ]));
    let body = json_to_string(&msg);
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    if let Ok(mut w) = writer.lock() {
        let _ = w.write_all(header.as_bytes());
        let _ = w.write_all(body.as_bytes());
        let _ = w.flush();
    }
}

/// Type for request handler functions.
pub type RequestHandler = Box<dyn Fn(&JsonValue) -> JsonValue + Send + Sync>;
pub type NotificationHandler = Box<dyn Fn(&JsonValue) + Send + Sync>;

/// A simple LSP server that reads from stdin and writes to stdout.
pub struct LspServer {
    pub client: Client,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    request_handlers: HashMap<String, RequestHandler>,
    notification_handlers: HashMap<String, NotificationHandler>,
}

impl LspServer {
    pub fn new() -> Self {
        let writer: Box<dyn Write + Send> = Box::new(std::io::stdout());
        let writer = Arc::new(Mutex::new(writer));
        let client = Client { writer: Arc::clone(&writer) };
        LspServer {
            client,
            writer,
            request_handlers: HashMap::new(),
            notification_handlers: HashMap::new(),
        }
    }

    pub fn on_request(&mut self, method: &str, handler: RequestHandler) {
        self.request_handlers.insert(method.to_string(), handler);
    }

    pub fn on_notification(&mut self, method: &str, handler: NotificationHandler) {
        self.notification_handlers.insert(method.to_string(), handler);
    }

    /// Run the server, reading from stdin until shutdown.
    pub fn run(self) {
        let stdin = std::io::stdin();
        let mut reader = BufReader::new(stdin.lock());

        loop {
            let body = match read_message(&mut reader) {
                Some(b) => b,
                None => break, // EOF
            };

            let msg = match parse_rpc(&body) {
                Some(m) => m,
                None => continue,
            };

            if msg.method == "shutdown" {
                if let Some(id) = msg.id {
                    send_response(&self.writer, id, JsonValue::Null);
                }
                continue;
            }

            if msg.method == "exit" {
                break;
            }

            if let Some(id) = msg.id {
                // Request — needs a response
                if let Some(handler) = self.request_handlers.get(&msg.method) {
                    let result = handler(&msg.params);
                    send_response(&self.writer, id, result);
                } else {
                    send_error(&self.writer, id, -32601, &format!("method not found: {}", msg.method));
                }
            } else {
                // Notification — no response
                if let Some(handler) = self.notification_handlers.get(&msg.method) {
                    handler(&msg.params);
                }
            }
        }
    }
}
