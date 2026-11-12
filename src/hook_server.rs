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
//!
//! Den Port erreicht jeder Prozess des Rechners. Deshalb wird das Token vor dem Body geprüft, und
//! Zeilen, Header, Body, Verbindungen und Dauer einer Anfrage sind begrenzt — ein Absturz in einem
//! Anfrage-Thread beendet wegen `panic = "abort"` die ganze App.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;

use crate::{hooks, trace};

/// Header mit der Claude-ID, unter der claude-tui die Session führt.
pub const SESSION_HEADER: &str = "X-Claude-Tui-Session";
/// Größter angenommener Body; `PreToolUse` enthält z. B. den ganzen Inhalt geschriebener Dateien.
const MAX_BODY: usize = 64 * 1024 * 1024;
/// Längste angenommene Zeile (Anfragezeile, Header, Chunk-Größe).
const MAX_LINE: u64 = 8 * 1024;
const MAX_HEADERS: usize = 64;
/// Gleichzeitig bearbeitete Verbindungen; weitere werden ohne Antwort geschlossen.
const MAX_CONNECTIONS: usize = 32;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Obergrenze für eine ganze Anfrage — tröpfelnde Absender belegen sonst dauerhaft Plätze.
const REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// Startet den Server im Hintergrund und liefert die URL für die Hook-Einstellungen; `None`, wenn
/// sich kein Port öffnen lässt (dann bleibt es bei den Befehls-Hooks).
pub fn start() -> Option<String> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).ok()?;
    let port = listener.local_addr().ok()?.port();
    let token = uuid::Uuid::new_v4().simple().to_string();
    let url = format!("http://127.0.0.1:{port}/hook/{token}");
    let path: Arc<str> = format!("/hook/{token}").into();
    let active = Arc::new(AtomicUsize::new(0));
    thread::Builder::new()
        .name("hook-server".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                // Über der Grenze fällt die Verbindung mit `stream`, ohne eigenen Thread.
                let Some(slot) = Slot::take(&active) else {
                    continue;
                };
                let path = Arc::clone(&path);
                // Scheitert der Start, fällt die Closure samt `slot` — der Zähler stimmt weiter.
                let _ = thread::Builder::new()
                    .name("hook-request".into())
                    .spawn(move || {
                        let _slot = slot;
                        handle(stream, &path);
                    });
            }
        })
        .ok()?;
    trace::mark(format_args!("hook server on port {port}"));
    Some(url)
}

/// Belegter Verbindungsplatz; wird beim Fallenlassen frei.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(active: &Arc<AtomicUsize>) -> Option<Self> {
        // Erst belegen; ist die Grenze überschritten, gibt `Drop` den Platz gleich wieder frei.
        let slot = Self(Arc::clone(active));
        (active.fetch_add(1, Ordering::AcqRel) < MAX_CONNECTIONS).then_some(slot)
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn handle(stream: TcpStream, expected_path: &str) {
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let reader = Deadline {
        inner: stream,
        until: Instant::now() + REQUEST_DEADLINE,
    };
    let status = match read_request(BufReader::new(reader), expected_path) {
        Ok(request) => {
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
                    hooks::receive(&event, &input, &key);
                }
            }
            "200 OK"
        }
        Err(Reject::NotFound) => "404 Not Found",
        Err(Reject::BadRequest) => "400 Bad Request",
    };
    // Leerer 2xx-Body heißt für Claude: Erfolg, keine Anweisungen.
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = writer.write_all(response.as_bytes());
    let _ = writer.flush();
}

/// Lesen mit Gesamtfrist: der Timeout des Sockets gilt nur je Lesevorgang.
struct Deadline<R> {
    inner: R,
    until: Instant,
}

impl<R: Read> Read for Deadline<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if Instant::now() >= self.until {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        self.inner.read(buf)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Request {
    session: Option<String>,
    body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
enum Reject {
    /// Falscher Pfad (Token) oder falsche Methode — der Body wird gar nicht erst gelesen.
    NotFound,
    BadRequest,
}

/// Liest eine HTTP/1.1-Anfrage: Zeile, Header, Body per `Content-Length` oder `chunked`. Methode und
/// Pfad werden vor den Headern geprüft, damit ohne Token kein Speicher für einen Body anfällt.
fn read_request(mut reader: impl BufRead, expected_path: &str) -> Result<Request, Reject> {
    let line = read_line(&mut reader)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().ok_or(Reject::BadRequest)?;
    let path = parts.next().ok_or(Reject::BadRequest)?;
    if method != "POST" || path != expected_path {
        return Err(Reject::NotFound);
    }
    let mut length = None;
    let mut chunked = false;
    let mut session = None;
    for _ in 0..=MAX_HEADERS {
        let line = read_line(&mut reader)?;
        if line.is_empty() {
            let body = if chunked {
                read_chunked(&mut reader)?
            } else {
                read_body(&mut reader, length.unwrap_or(0))?
            };
            return Ok(Request { session, body });
        }
        let (name, value) = line.split_once(':').ok_or(Reject::BadRequest)?;
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.parse::<usize>().map_err(|_| Reject::BadRequest)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        } else if name.eq_ignore_ascii_case(SESSION_HEADER) && !value.is_empty() {
            session = Some(value.to_string());
        }
    }
    Err(Reject::BadRequest)
}

/// Eine Zeile ohne Zeilenende; höchstens [`MAX_LINE`] Bytes, sonst ungültig.
fn read_line(reader: &mut impl BufRead) -> Result<String, Reject> {
    let mut line = String::new();
    reader
        .by_ref()
        .take(MAX_LINE)
        .read_line(&mut line)
        .map_err(|_| Reject::BadRequest)?;
    if !line.ends_with('\n') {
        return Err(Reject::BadRequest);
    }
    Ok(line.trim_end().to_string())
}

/// Genau `length` Bytes. Der Puffer wächst mit den gelesenen Daten, statt vorab reserviert zu
/// werden — eine Längenangabe allein belegt keinen Speicher.
fn read_body(reader: &mut impl BufRead, length: usize) -> Result<Vec<u8>, Reject> {
    if length > MAX_BODY {
        return Err(Reject::BadRequest);
    }
    let mut body = Vec::new();
    reader
        .by_ref()
        .take(length as u64)
        .read_to_end(&mut body)
        .map_err(|_| Reject::BadRequest)?;
    if body.len() != length {
        return Err(Reject::BadRequest);
    }
    Ok(body)
}

fn read_chunked(reader: &mut impl BufRead) -> Result<Vec<u8>, Reject> {
    let mut body = Vec::new();
    loop {
        let line = read_line(reader)?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| Reject::BadRequest)?;
        if size == 0 {
            // Abschließende (leere) Trailer-Zeile.
            let _ = read_line(reader);
            return Ok(body);
        }
        if body
            .len()
            .checked_add(size)
            .is_none_or(|total| total > MAX_BODY)
        {
            return Err(Reject::BadRequest);
        }
        body.extend_from_slice(&read_body(reader, size)?);
        if !read_line(reader)?.is_empty() {
            return Err(Reject::BadRequest);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PATH: &str = "/hook/abc";

    #[test]
    fn request_with_content_length_is_parsed() {
        let raw = b"POST /hook/abc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nx-claude-tui-session: s-1\r\nContent-Length: 2\r\n\r\n{}";
        let request = read_request(&raw[..], PATH).expect("Anfrage");
        assert_eq!(request.session.as_deref(), Some("s-1"));
        assert_eq!(request.body, b"{}");
    }

    #[test]
    fn chunked_body_is_joined() {
        let raw = b"POST /hook/abc HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n{\"a\r\n4\r\n\":1}\r\n0\r\n\r\n";
        let request = read_request(&raw[..], PATH).expect("Anfrage");
        assert_eq!(request.body, b"{\"a\":1}");
        assert_eq!(request.session, None);
    }

    #[test]
    fn get_wrong_token_and_garbage_are_rejected() {
        let get = b"GET /hook/abc HTTP/1.1\r\n\r\n";
        assert_eq!(read_request(&get[..], PATH), Err(Reject::NotFound));
        let wrong = b"POST /hook/other HTTP/1.1\r\n\r\n";
        assert_eq!(read_request(&wrong[..], PATH), Err(Reject::NotFound));
        assert_eq!(read_request(&b"\r\n"[..], PATH), Err(Reject::BadRequest));
        assert_eq!(read_request(&b""[..], PATH), Err(Reject::BadRequest));
    }

    #[test]
    fn wrong_token_is_rejected_before_the_body_is_read() {
        // Riesige Längenangabe ohne Body: ohne Token kein Lesen, kein Speicher.
        let raw = b"POST /hook/other HTTP/1.1\r\nContent-Length: 60000000\r\n\r\n";
        assert_eq!(read_request(&raw[..], PATH), Err(Reject::NotFound));
    }

    #[test]
    fn overflowing_chunk_size_is_rejected_without_panic() {
        let raw = b"POST /hook/abc HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\nffffffffffffffff\r\n";
        assert_eq!(read_request(&raw[..], PATH), Err(Reject::BadRequest));
    }

    #[test]
    fn oversized_body_and_short_body_are_rejected() {
        let too_big = format!(
            "POST /hook/abc HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert_eq!(
            read_request(too_big.as_bytes(), PATH),
            Err(Reject::BadRequest)
        );
        let short = b"POST /hook/abc HTTP/1.1\r\nContent-Length: 10\r\n\r\n{}";
        assert_eq!(read_request(&short[..], PATH), Err(Reject::BadRequest));
    }

    #[test]
    fn overlong_lines_and_too_many_headers_are_rejected() {
        let long_line = format!(
            "POST /hook/abc HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(10_000)
        );
        assert_eq!(
            read_request(long_line.as_bytes(), PATH),
            Err(Reject::BadRequest)
        );
        let many = format!(
            "POST /hook/abc HTTP/1.1\r\n{}\r\n",
            "X: y\r\n".repeat(MAX_HEADERS + 1)
        );
        assert_eq!(read_request(many.as_bytes(), PATH), Err(Reject::BadRequest));
    }

    #[test]
    fn connection_slots_are_limited_and_freed() {
        let active = Arc::new(AtomicUsize::new(0));
        let slots: Vec<Slot> = (0..MAX_CONNECTIONS)
            .map(|_| Slot::take(&active).expect("Platz frei"))
            .collect();
        assert!(Slot::take(&active).is_none());
        drop(slots);
        assert_eq!(active.load(Ordering::Acquire), 0);
        assert!(Slot::take(&active).is_some());
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
