//! Tiny HTTP endpoint on 127.0.0.1 for the Island API, on plain sockets so it never needs admin rights or URL
//! reservations. Requests from web pages are refused (they carry an Origin header, or a foreign Host after DNS
//! rebinding), so only programs on this PC, such as scripts and curl, can talk to it.

use serde::Serialize;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A request handed to the handler; the JSON body is `{}` when empty.
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Value,
}

/// What a handler answers: HTTP status and a JSON body.
#[derive(Clone, Debug, PartialEq)]
pub struct Response {
    pub status: u16,
    pub json: String,
}

impl Response {
    pub fn ok() -> Response {
        Response { status: 200, json: r#"{"ok":true}"#.into() }
    }

    pub fn json(status: u16, value: &impl Serialize) -> Response {
        Response { status, json: serde_json::to_string(value).unwrap_or_else(|_| "{}".into()) }
    }

    pub fn error(status: u16, message: &str) -> Response {
        #[derive(Serialize)]
        struct Error<'a> {
            error: &'a str,
        }
        Response::json(status, &Error { error: message })
    }
}

/// Listens until dropped.
pub struct Server {
    stop: Arc<AtomicBool>,
    port: u16,
}

impl Server {
    /// `handler` answers a request, or returns `None` for "not found". Called on a background thread.
    pub fn start(port: u16, handler: impl Fn(Request) -> Option<Response> + Send + Sync + 'static) -> Option<Server> {
        let listener = match TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))) {
            Ok(l) => l,
            Err(e) => {
                di4w_core::log::error_with(format!("Could not listen on port {port}"), e);
                return None;
            }
        };
        di4w_core::log::info(format!("Local server listening on http://127.0.0.1:{port}/"));
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for client in listener.incoming() {
                if flag.load(Ordering::SeqCst) {
                    break;
                }
                match client {
                    Ok(stream) => {
                        let h = handler.clone();
                        std::thread::spawn(move || {
                            let _ = serve(stream, &*h);
                        });
                    }
                    Err(e) => di4w_core::log::error_with("Accept failed", e),
                }
            }
        });
        Some(Server { stop, port })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, self.port)), Duration::from_millis(200));
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Internal Server Error",
    }
}

fn serve(mut stream: TcpStream, handler: &dyn Fn(Request) -> Option<Response>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let raw = read_request(&mut stream)?;
    let response = answer(raw, handler);
    let body = response.json.as_bytes();
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

pub struct Raw {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Raw {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().rev().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// Checks the request and hands it to the handler.
pub fn answer(raw: Raw, handler: &dyn Fn(Request) -> Option<Response>) -> Response {
    // Browsers add Origin to cross-site requests, and a DNS-rebinding page would arrive with its own Host.
    if raw.header("Origin").is_some() {
        return Response::error(403, "Requests from web pages are not allowed.");
    }
    if let Some(host) = raw.header("Host")
        && !is_local_host(host) {
            return Response::error(403, "Unexpected Host header.");
        }
    let body = if raw.body.trim().is_empty() { "{}" } else { raw.body.as_str() };
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return Response::error(400, "The body isn't valid JSON.");
    };
    let (method, path) = (raw.method.clone(), raw.path.clone());
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler(Request { method: raw.method, path: raw.path, body: json }))) {
        Ok(Some(r)) => r,
        Ok(None) => Response::error(404, "No such endpoint."),
        Err(_) => {
            di4w_core::log::error(format!("Local request {method} {path} failed"));
            Response::error(500, "The island couldn't handle the request.")
        }
    }
}

fn is_local_host(host: &str) -> bool {
    let name = match host.rfind(':') {
        Some(i) if !host.ends_with(']') => &host[..i],
        _ => host,
    };
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Raw> {
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    let header_end = loop {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        data.extend_from_slice(&buffer[..n]);
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if data.len() > 64 * 1024 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let first: Vec<&str> = lines.next().unwrap_or_default().split(' ').collect();
    let method = first.first().unwrap_or(&"").to_uppercase();
    let path = first.get(1).map_or("/", |p| p.split('?').next().unwrap_or("/")).to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| {
            let i = line.find(':')?;
            (i > 0).then(|| (line[..i].trim().to_string(), line[i + 1..].trim().to_string()))
        })
        .collect();
    let length = headers
        .iter()
        .rev()
        .find(|(k, _)| k.eq_ignore_ascii_case("Content-Length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0)
        .min(1024 * 1024);
    let start = header_end + 4;
    while data.len() - start < length {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..n]);
    }
    let end = (start + length).min(data.len());
    Ok(Raw { method, path, headers, body: String::from_utf8_lossy(&data[start..end]).to_string() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(headers: &[(&str, &str)], body: &str) -> Raw {
        Raw { method: "POST".into(), path: "/alert".into(), headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), body: body.into() }
    }

    #[test]
    fn refuses_web_pages() {
        let echo = |_: Request| Some(Response::ok());
        assert_eq!(answer(raw(&[("Origin", "https://evil.example")], "{}"), &echo).status, 403);
        assert_eq!(answer(raw(&[("Host", "evil.example:47823")], "{}"), &echo).status, 403);
        assert_eq!(answer(raw(&[("Host", "127.0.0.1:47823")], "{}"), &echo).status, 200);
        assert_eq!(answer(raw(&[("Host", "localhost")], ""), &echo).status, 200);
    }

    #[test]
    fn rejects_bad_json_and_unknown_paths() {
        assert_eq!(answer(raw(&[], "{nope"), &|_| Some(Response::ok())).status, 400);
        assert_eq!(answer(raw(&[], "{}"), &|_| None).json, r#"{"error":"No such endpoint."}"#);
    }
}
