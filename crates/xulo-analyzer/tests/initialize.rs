//! End-to-end handshake tests: the server is spawned over stdio exactly the
//! way `editors/vscode/extension.js` talks to it, so a capability shape a real
//! client can send must never crash the handshake.
//!
//! The `initialize` payload used to be parsed with `expect`, which aborted the
//! whole process the moment a client sent an unfamiliar field type.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    incoming: Receiver<Value>,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_xulo-analyzer"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn xulo-analyzer");
        let stdin = child.stdin.take().expect("piped stdin");

        let stdout = child.stdout.take().expect("piped stdout");
        let mut stderr = child.stderr.take().expect("piped stderr");
        thread::spawn(move || {
            let mut sink = Vec::new();
            let _ = std::io::copy(&mut stderr, &mut sink);
        });

        let (tx, incoming) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            while let Some(frame) = read_frame(&mut reader) {
                if tx.send(frame).is_err() {
                    break;
                }
            }
        });

        Self {
            child,
            stdin: Some(stdin),
            incoming,
        }
    }

    fn send(&mut self, message: &Value) {
        let body = serde_json::to_vec(message).expect("serialize message");
        self.write_frame(&body)
            .unwrap_or_else(|e| panic!("write to xulo-analyzer: {e}"));
    }

    /// Best-effort write, used on the teardown path where the server may
    /// already be gone — a panic inside `Drop` would abort the test run.
    fn try_send(&mut self, message: &Value) {
        if let Ok(body) = serde_json::to_vec(message) {
            let _ = self.write_frame(&body);
        }
    }

    fn write_frame(&mut self, body: &[u8]) -> std::io::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| std::io::Error::other("stdin already closed"))?;
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len())?;
        stdin.write_all(body)?;
        stdin.flush()
    }

    /// Close the server's stdin, exactly like a client process dying.
    fn disconnect(&mut self) {
        self.stdin.take();
    }

    /// Wait for the server to exit on its own; `None` while it is alive.
    fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return Some(status),
                Ok(None) if std::time::Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20));
                }
                _ => return None,
            }
        }
    }

    fn recv(&self) -> Value {
        self.incoming
            .recv_timeout(Duration::from_secs(15))
            .expect("server must answer")
    }

    /// Send `initialize` with `extra` merged into its params and return the
    /// whole response (result or error).
    fn initialize(&mut self, capabilities: Value, extra: Value) -> Value {
        let mut params = json!({ "processId": null, "capabilities": capabilities });
        merge(&mut params, extra);
        self.send(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": params
        }));
        let response = self.recv();
        assert_eq!(
            response["id"],
            Value::from(1),
            "response is for `initialize`"
        );
        // `initialize_finish` blocks until the client acknowledges, so the
        // handshake is only complete after this notification.
        self.send(&json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }));
        response
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.try_send(&json!({ "jsonrpc": "2.0", "method": "exit", "params": Value::Null }));
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if std::time::Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

/// Read one `Content-Length`-framed JSON-RPC message; `None` at EOF.
fn read_frame<R: BufRead>(reader: &mut R) -> Option<Value> {
    let mut content_length = None;
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return None,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("Content-Length:") {
            content_length = rest.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0u8; content_length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn merge(target: &mut Value, extra: Value) {
    if let (Some(target), Some(extra)) = (target.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
}

/// The capability payload `extension.js` sends.
fn vscode_client_capabilities() -> Value {
    json!({
        "workspace": {
            "applyEdit": false,
            "workspaceFolders": true,
            "configuration": true
        },
        "textDocument": {
            "synchronization": { "didSave": false },
            "publishDiagnostics": { "relatedInformation": true, "tagSupport": { "valueSet": [1, 2] } },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "definition": { "linkSupport": false },
            "references": {},
            "documentSymbol": { "hierarchicalDocumentSymbolSupport": true },
            "formatting": {},
            "semanticTokens": {
                "requests": { "range": false, "full": true },
                "tokenTypes": ["variable", "parameter", "function"],
                "tokenModifiers": ["declaration"],
                "formats": ["relative"]
            }
        },
        "window": { "workDoneProgress": true }
    })
}

#[test]
fn initialize_advertises_every_feature_the_extension_registers_providers_for() {
    let mut server = Server::start();
    let response = server.initialize(
        vscode_client_capabilities(),
        json!({
            "processId": 1,
            "clientInfo": { "name": "vscode-xulo", "version": "0.3.0" },
            "rootUri": Value::Null,
            "workspaceFolders": Value::Null
        }),
    );
    let caps = &response["result"]["capabilities"];
    assert!(caps.is_object(), "initialize must succeed: {response}");
    for provider in [
        "definitionProvider",
        "hoverProvider",
        "referencesProvider",
        "documentSymbolProvider",
        "documentFormattingProvider",
        "semanticTokensProvider",
    ] {
        assert!(
            caps[provider].is_object() || caps[provider] == true,
            "missing `{provider}`"
        );
    }
    assert_eq!(
        caps["semanticTokensProvider"]["legend"]["tokenModifiers"],
        json!(["declaration"])
    );
    assert!(server.is_alive(), "server must stay up after initialize");
}

#[test]
fn initialize_survives_unfamiliar_capability_shapes() {
    // The first entry is the shape lsp-types 0.94 mis-types as a boolean and
    // the object form the LSP spec actually prescribes; the rest are payloads
    // a buggy or newer client could send. None may abort the process.
    let cases = [
        json!({ "workspace": { "workspaceFolders": { "supported": true, "changeNotifications": true } } }),
        json!({ "workspace": { "workspaceFolders": true } }),
        json!({ "workspace": { "workspaceFolders": "yes" } }),
        json!({ "textDocument": { "hover": { "contentFormat": "markdown" } } }),
        json!({ "textDocument": "nope" }),
        json!({ "window": { "workDoneProgress": 42 } }),
        json!("not even an object"),
        json!(null),
    ];
    for (index, capabilities) in cases.into_iter().enumerate() {
        let mut server = Server::start();
        let response = server.initialize(
            capabilities,
            json!({
                "processId": "not-a-number",
                "clientInfo": 7,
                "rootUri": 3
            }),
        );
        assert!(
            response["result"].is_object() || response["error"].is_object(),
            "case {index} must get a JSON-RPC reply, got: {response}"
        );
        // The handshake continues: an unknown method is answered, not dropped.
        server.send(&json!({ "jsonrpc": "2.0", "id": 9, "method": "nope/nope", "params": {} }));
        let reply = server.recv();
        assert_eq!(
            reply["id"],
            Value::from(9),
            "case {index} keeps serving: {reply}"
        );
        assert!(
            reply["error"].is_object(),
            "case {index} answers unknown methods with MethodNotFound: {reply}"
        );
        assert!(server.is_alive(), "case {index} must not crash");
    }
}

#[test]
fn client_disconnecting_mid_handshake_exits_cleanly() {
    // A real editor can die between the `initialize` response and its
    // `initialized` ack. The server used to `.expect(...)` there and abort
    // with code 101; it must now treat it as an ordinary shutdown.
    let mut server = Server::start();
    let response = server.initialize(
        vscode_client_capabilities(),
        json!({ "processId": 1, "rootUri": Value::Null }),
    );
    assert!(
        response["result"].is_object(),
        "initialize must succeed: {response}"
    );
    server.disconnect();
    let status = server
        .wait_exit(Duration::from_secs(15))
        .expect("server must exit after the client disappears");
    assert!(
        status.success(),
        "graceful shutdown, not a panic: {status:?}"
    );
}
