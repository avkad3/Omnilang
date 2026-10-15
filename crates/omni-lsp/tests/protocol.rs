//! A full LSP session over an in-memory connection.

use std::thread;
use std::time::Duration;

use lsp_server::{Connection, Message, Notification, Request, RequestId};
use serde_json::{json, Value};

struct Client {
    conn: Connection,
    next_id: i32,
}

impl Client {
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.conn
            .sender
            .send(Message::Request(Request::new(id.clone(), method.into(), params)))
            .unwrap();
        loop {
            match self.recv() {
                Message::Response(r) if r.id == id => match r.response_result {
                    Ok(value) => return value,
                    Err(e) => panic!("{method}: {} ({})", e.message, e.code),
                },
                _ => continue, // notifications such as diagnostics
            }
        }
    }

    fn notify(&self, method: &str, params: Value) {
        self.conn
            .sender
            .send(Message::Notification(Notification::new(method.into(), params)))
            .unwrap();
    }

    fn recv(&self) -> Message {
        self.conn
            .receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("server replied")
    }

    /// Waits for the next diagnostics notification.
    fn diagnostics(&self) -> Value {
        loop {
            if let Message::Notification(n) = self.recv() {
                if n.method == "textDocument/publishDiagnostics" {
                    return n.params;
                }
            }
        }
    }
}

const URI: &str = "file:///tmp/omni-test/prog.om";

#[test]
fn hindi_session_end_to_end() {
    let (server, client) = Connection::memory();
    let handle = thread::spawn(move || omni_lsp::serve(server).unwrap());
    let mut c = Client {
        conn: client,
        next_id: 0,
    };

    let init = c.request(
        "initialize",
        json!({ "capabilities": {}, "initializationOptions": { "messageLanguage": "hi", "typingLanguage": "hi" } }),
    );
    assert_eq!(init["capabilities"]["completionProvider"], json!({}));
    assert_eq!(init["serverInfo"]["name"], "omni");
    c.notify("initialized", json!({}));

    // An undefined name: the error arrives in Hindi at the right position.
    let text = "कार्य मुख्य() {\n    गिनती छापो।\n}\n";
    c.notify(
        "textDocument/didOpen",
        json!({ "textDocument": { "uri": URI, "languageId": "omnilang", "version": 1, "text": text } }),
    );
    let d = c.diagnostics();
    assert_eq!(d["diagnostics"][0]["code"], "E0201");
    assert!(d["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains("परिभाषित नहीं"));
    assert_eq!(
        d["diagnostics"][0]["range"]["start"],
        json!({ "line": 1, "character": 4 })
    );

    // Typing "ret" completes to the Hindi keyword.
    let text = "कार्य मुख्य() {\n    5 ret";
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 2 }, "contentChanges": [{ "text": text }] }),
    );
    c.diagnostics();
    let list = c.request(
        "textDocument/completion",
        json!({ "textDocument": { "uri": URI }, "position": { "line": 1, "character": 9 } }),
    );
    assert_eq!(list["isIncomplete"], true);
    assert_eq!(list["items"][0]["label"], "लौटाओ");

    // Hover on a keyword lists every language.
    let text = "कार्य मुख्य() {\n    1 छापो।\n}\n";
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 3 }, "contentChanges": [{ "text": text }] }),
    );
    assert_eq!(c.diagnostics()["diagnostics"], json!([]));
    let h = c.request(
        "textDocument/hover",
        json!({ "textDocument": { "uri": URI }, "position": { "line": 1, "character": 7 } }),
    );
    let md = h["contents"]["value"].as_str().unwrap();
    assert!(md.contains("print x;") && md.contains("x அச்சிடு;"), "{md}");

    // The whole document, shown in Tamil.
    let tamil = c.request(
        "workspace/executeCommand",
        json!({ "command": "omni.view", "arguments": [URI, "ta"] }),
    );
    assert_eq!(tamil, "செயல்பாடு முதன்மை() {\n    1 அச்சிடு;\n}\n");

    // Switching the message language re-publishes diagnostics in Tamil.
    c.notify(
        "textDocument/didChange",
        json!({ "textDocument": { "uri": URI, "version": 4 }, "contentChanges": [{ "text": "कार्य मुख्य() { x छापो। }" }] }),
    );
    c.diagnostics();
    c.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "omni": { "messageLanguage": "ta", "typingLanguage": "ta" } } }),
    );
    let d = c.diagnostics();
    assert!(d["diagnostics"][0]["message"]
        .as_str()
        .unwrap()
        .contains("வரையறுக்கப்படவில்லை"));

    c.request("shutdown", Value::Null);
    c.notify("exit", Value::Null);
    handle.join().unwrap();
}
