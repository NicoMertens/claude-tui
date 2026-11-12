//! Empfang der Hook-Meldungen per HTTP: claude-tui lauscht auf `127.0.0.1:<freier Port>`, Claude
//! Code schickt jedes Hook-Event als `POST` mit dem Event-JSON (Hook-Typ `http`).
//!
//! Warum nicht `claude-tui hook <Event>` als Befehl: Claude startet dafür je Event einen Prozess.
//! Prüft ein Virenscanner die unsignierte Binary bei jedem Start, dauert das Sekunden — und schon
//! das Anlegen des Prozesses hält Claude auf, auch bei `async`. Eine HTTP-Anfrage an localhost
//! kostet dagegen Millisekunden.
//!
//! Der Pfad enthält ein zufälliges Token je Lauf, damit andere Programme keine Zustände unterschieben
//! können. Die Session steht im Header `X-Claude-Tui-Session` (aus `CLAUDE_TUI_SESSION`), sonst
//! gilt `session_id` aus dem JSON.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    thread,
    time::Duration,
};

use serde_json::Value;

use crate::{hooks, trace};

/// Header mit der Claude-ID, unter der claude-tui die Session führt.
pub const SESSION_HEADER: &str = "X-Claude-Tui-Session";
/// Größter angenommener Body; `PreToolUse` enthält z. B. den ganzen Inhalt geschriebener Dateien.
const MAX_BODY: usize = 64 * 1024 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// Startet den Server im Hintergrund und liefert die URL für die Hook-Einstellungen; `None`, wenn
/// sich kein Port öffnen lässt (dann bleibt es bei den Befehls-Hooks).
pub fn start() -> Option<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let url = format!("http://127.0.0.1:{port}/hook/{token}");
    let path: Arc<str> = format!("/hook/{token}").into();
    thread::Builder::new()
        .name("hook-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let path = Arc::clone(&path);
                let _ = thread::Builder::new()
                    .name("hook-request".into())
                    .spawn(move || handle(stream, &path));
            }
        })
        .ok()?;
    trace::mark(format_args!("hook server on port {port}"));
    Some(url)
}

fn handle(stream: TcpStream, expected_path: &str) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let status = match read_request(BufReader::new(stream)) {
        Some(request) if request.path == expected_path => {
            if let Ok(input) = serde_json::from_slice::<Value>(&request.body) {
                let event = input
                    .get("hook_event_name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let key = request.session.or_else(|| {
                    input
                        .get("session_id")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
                if let Some(key) = key {
                    trace::mark(format_args!("hook {event} for {key} (http)"));
                    hooks::apply(&event, &input, &key);
                }
            }
            "200 OK"
        }
        Some(_) => "404 Not Found",
        None => "400 Bad Request",
    };
    // Leerer 2xx-Body heißt für Claude: Erfolg, keine Anweisungen.
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = writer.write_all(response.as_bytes());
    let _ = writer.flush();
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    path: String,
    session: Option<String>,
    body: Vec<u8>,
}

/// Liest eine HTTP/1.1-Anfrage: Zeile, Header, Body per `Content-Length` oder `chunked`.
fn read_request(mut reader: impl BufRead) -> Option<Request> {
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?;
    let path = parts.next()?.to_string();
    if method != "POST" {
        return None;
    }
    let mut length = None;
    let mut chunked = false;
    let mut session = None;
    loop {
        line.clear();
        if reader.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        let (name, value) = header.split_once(':')?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = value.parse::<usize>().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        } else if name.eq_ignore_ascii_case(SESSION_HEADER) && !value.is_empty() {
            session = Some(value.to_string());
        }
    }
    let body = if chunked {
        read_chunked(&mut reader)?
    } else {
        let length = length.unwrap_or(0);
        if length > MAX_BODY {
            return None;
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        body
    };
    Some(Request {
        path,
        session,
        body,
    })
}

fn read_chunked(reader: &mut impl BufRead) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    let mut line = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).ok()?;
        let size_text = line.trim().split(';').next()?;
        let size = usize::from_str_radix(size_text, 16).ok()?;
        if size == 0 {
            // Abschließende (leere) Trailer-Zeile.
            line.clear();
            let _ = reader.read_line(&mut line);
            return Some(body);
        }
        if body.len() + size > MAX_BODY {
            return None;
        }
        let start = body.len();
        body.resize(start + size, 0);
        reader.read_exact(&mut body[start..]).ok()?;
        line.clear();
        reader.read_line(&mut line).ok()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn request_with_content_length_is_parsed() {
        let raw = b"POST /hook/abc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nx-claude-tui-session: s-1\r\nContent-Length: 2\r\n\r\n{}";
        let request = read_request(&raw[..]).expect("Anfrage");
        assert_eq!(request.path, "/hook/abc");
        assert_eq!(request.session.as_deref(), Some("s-1"));
        assert_eq!(request.body, b"{}");
    }

    #[test]
    fn chunked_body_is_joined() {
        let raw = b"POST /hook/abc HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        let request = read_request(&raw[..]).expect("Anfrage");
        assert_eq!(request.body, b"{\"a\":1}");
        assert_eq!(request.session, None);
    }

    #[test]
    fn get_and_garbage_are_rejected() {
        assert_eq!(read_request(&b"GET /hook/abc HTTP/1.1\r\n\r\n"[..]), None);
        assert_eq!(read_request(&b"\r\n"[..]), None);
    }

    #[test]
    fn server_answers_and_rejects_wrong_token() {
        let url = start().expect("Server");
        let rest = url.trim_start_matches("http://");
        let (addr, path) = rest.split_once('/').expect("Pfad");
        let send = |path: &str| {
            let mut stream = TcpStream::connect(addr).expect("Verbindung");
            let body = r#"{"hook_event_name":"Test"}"#;
            let request = format!(
                "POST {path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(request.as_bytes()).expect("senden");
            let mut answer = String::new();
            stream.read_to_string(&mut answer).expect("lesen");
            answer
        };
        assert!(send(&format!("/{path}")).starts_with("HTTP/1.1 200"));
        assert!(send("/hook/falsch").starts_with("HTTP/1.1 404"));
    }
}
