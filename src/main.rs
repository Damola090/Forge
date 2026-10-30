mod handler;
mod middleware;
mod request;
mod response;
mod router;

use handler::{FnHandler, Handler};
use middleware::{LoggingMiddleware, MiddlewareChain, RequestIdMiddleware, TimingMiddleware};
use request::Request;
use response::{Response, StatusCode};
use router::Router;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use std::thread;
use std::time::Duration;

fn build_router() -> Router {
    let mut router = Router::new();

    // Wrap each handler in a middleware chain
    // Every route gets logging, timing, and request ID automatically
    // The handler itself knows nothing about any of this

    router.get(
        "/",
        Box::new(
            MiddlewareChain::new(Box::new(FnHandler::new(|_req, _params| {
                Response::ok().body("<h1>Welcome to Forge</h1>", "text/html")
            })))
            .with(Box::new(LoggingMiddleware))
            .with(Box::new(TimingMiddleware))
            .with(Box::new(RequestIdMiddleware)),
        ),
    );

    router.get(
        "/health",
        Box::new(
            MiddlewareChain::new(Box::new(FnHandler::new(|_req, _params| {
                Response::ok().body(r#"{"status": "ok"}"#, "application/json")
            })))
            .with(Box::new(LoggingMiddleware))
            .with(Box::new(TimingMiddleware))
            .with(Box::new(RequestIdMiddleware)),
        ),
    );

    router.get(
        "/media/:id",
        Box::new(
            MiddlewareChain::new(Box::new(FnHandler::new(|_req, params| {
                let id = params.get("id").map(|s| s.as_str()).unwrap_or("unknown");
                let body = format!(r#"{{"id": "{}", "status": "found"}}"#, id);
                Response::ok().body(&body, "application/json")
            })))
            .with(Box::new(LoggingMiddleware))
            .with(Box::new(TimingMiddleware))
            .with(Box::new(RequestIdMiddleware)),
        ),
    );

    router.post(
        "/media/upload",
        Box::new(
            MiddlewareChain::new(Box::new(FnHandler::new(|req, _params| {
                let size = req.body.len();
                let body = format!(r#"{{"received": "{} bytes"}}"#, size);
                Response::ok().body(&body, "application/json")
            })))
            .with(Box::new(LoggingMiddleware))
            .with(Box::new(TimingMiddleware))
            .with(Box::new(RequestIdMiddleware)),
        ),
    );

    router.delete(
        "/media/:id",
        Box::new(
            MiddlewareChain::new(Box::new(FnHandler::new(|_req, params| {
                let id = params.get("id").map(|s| s.as_str()).unwrap_or("unknown");
                let body = format!(r#"{{"deleted": "{}"}}"#, id);
                Response::ok().body(&body, "application/json")
            })))
            .with(Box::new(LoggingMiddleware))
            .with(Box::new(TimingMiddleware))
            .with(Box::new(RequestIdMiddleware)),
        ),
    );

    router
}

// Limits so one client can't make us buffer unbounded data
const MAX_HEADER_SIZE: usize = 8 * 1024;
const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

enum ReadError {
    // Client closed the connection
    Closed,
    Io(std::io::Error),
    HeadersTooLarge,
    BodyTooLarge,
}

// Reads exactly one HTTP request off the socket
// A single read() is not enough — TCP is a stream, not messages
// A request can arrive split across many reads, and one read can
// contain the end of one request and the start of the next
//
// 1. Read until the blank line — now we have all the headers
// 2. Get Content-Length from the headers
// 3. Read until we have exactly that many body bytes
//
// `pending` holds bytes already read but not yet used
// Anything past the end of this request stays in it for the next call
async fn read_request(socket: &mut TcpStream, pending: &mut Vec<u8>) -> Result<Vec<u8>, ReadError> {
    let mut chunk = [0u8; 4096];

    // Step 1 — read until the headers are complete
    let (headers_end, body_start) = loop {
        if let Some(bounds) = request::header_end(pending) {
            break bounds;
        }
        if pending.len() > MAX_HEADER_SIZE {
            return Err(ReadError::HeadersTooLarge);
        }
        let n = socket.read(&mut chunk).await.map_err(ReadError::Io)?;
        if n == 0 {
            // 0 bytes = EOF = they're done
            return Err(ReadError::Closed);
        }
        pending.extend_from_slice(&chunk[..n]);
    };

    // Step 2 — how many body bytes should follow the headers
    let content_length = content_length(&pending[..headers_end]);
    if content_length > MAX_BODY_SIZE {
        return Err(ReadError::BodyTooLarge);
    }

    // Clients sending big bodies (curl over 1MB) ask permission first
    // and wait for 100 Continue before sending the body
    if expects_continue(&pending[..headers_end]) && pending.len() < body_start + content_length {
        socket
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")
            .await
            .map_err(ReadError::Io)?;
    }

    // Step 3 — read until the whole body has arrived
    let request_end = body_start + content_length;
    while pending.len() < request_end {
        let n = socket.read(&mut chunk).await.map_err(ReadError::Io)?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        pending.extend_from_slice(&chunk[..n]);
    }

    // Split off this request — whatever is left belongs to the next one
    let next = pending.split_off(request_end);
    Ok(std::mem::replace(pending, next))
}

// Find a header value in the raw header bytes, before full parsing
fn raw_header(header_bytes: &[u8], name: &str) -> Option<String> {
    String::from_utf8_lossy(header_bytes)
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_string())
}

fn content_length(header_bytes: &[u8]) -> usize {
    raw_header(header_bytes, "content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

fn expects_continue(header_bytes: &[u8]) -> bool {
    raw_header(header_bytes, "expect")
        .map(|v| v.eq_ignore_ascii_case("100-continue"))
        .unwrap_or(false)
}

// Handles one connection from start to finish
// This runs as an independent Tokio task
// Multiple of these run simultaneously
async fn handle_connection(mut socket: TcpStream, router: Arc<Router>) {
    let mut pending = Vec::new();

    // Loop — handle multiple requests on the same connection
    // This is keep-alive — the connection stays open
    loop {
        let raw = match read_request(&mut socket, &mut pending).await {
            Ok(raw) => raw,
            Err(ReadError::Closed) => {
                println!("Client disconnected");
                return;
            }
            Err(ReadError::Io(e)) => {
                eprintln!("Read error: {}", e);
                return;
            }
            Err(ReadError::HeadersTooLarge) => {
                let response = Response::new(StatusCode::RequestHeaderFieldsTooLarge)
                    .body("Request headers too large", "text/plain")
                    .header("Connection", "close");
                let _ = socket.write_all(&response.to_bytes()).await;
                return;
            }
            Err(ReadError::BodyTooLarge) => {
                let response = Response::new(StatusCode::PayloadTooLarge)
                    .body("Request body too large", "text/plain")
                    .header("Connection", "close");
                let _ = socket.write_all(&response.to_bytes()).await;
                return;
            }
        };

        let (response, should_close) = match Request::parse(&raw) {
            Ok(req) => {
                println!("{:?} {} — done", req.method, req.path);
                // Check if the client wants to close after this request
                // HTTP/1.1 default is keep-alive
                // but the client can send "Connection: close" to signal
                // they want the connection closed after this response
                let wants_close = req
                    .header("connection")
                    .map(|v| v.to_lowercase() == "close")
                    .unwrap_or(false);

                let mut response = router.handle(&req);

                // Tell the client whether we're keeping the connection open
                if wants_close {
                    response = response.header("Connection", "close");
                } else {
                    response = response.header("Connection", "keep-alive");
                }

                (response, wants_close)
            }
            Err(e) => {
                eprintln!("Parse error: {}", e);
                // On parse error, close the connection
                // We can't trust anything coming from this client
                let response =
                    Response::bad_request("Could not parse request").header("Connection", "close");
                (response, true)
            }
        };

        //Write the response
        if let Err(e) = socket.write_all(&response.to_bytes()).await {
            eprintln!("Write error: {}", e);
            return;
        }

        //close if requested
        if should_close {
            return;
        }

        // otherwise loop back and wait for the next request
        // same socket, same connection, no new TCP handshake
    }
}

#[tokio::main]
async fn main() {
    // Wrap the router in Arc so it can be shared across tasks
    // Arc::new moves the router onto the heap
    // Arc gives each task a reference-counted pointer to it
    let router = Arc::new(build_router());

    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Forge listening on port 8080");

    loop {
        let (socket, addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("Accept error: {}", e);
                continue;
            }
        };

        println!("Connection from {}", addr);

        // Clone the Arc — not the router
        // This increments the reference count by 1
        // When the task finishes, the count decrements
        // When count hits 0, the router is dropped
        let router = Arc::clone(&router);

        // Spawn an independent task for this connection
        // move takes ownership of socket and router_clone
        // the loop immediately continues to accept() again
        tokio::spawn(async move {
            handle_connection(socket, router).await;
        });

        // println!(
        //     "Responded: {} {} → {}",
        //     match request::Request::parse(&buffer[..bytes_read]) {
        //         Ok(r) => format!("{:?}", r.method),
        //         Err(_) => "??".to_string(),
        //     },
        //     "path",
        //     response.status as u16
        // );
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    // A connected pair of sockets on localhost
    // Whatever is written to `client` can be read by read_request from `server`
    async fn socket_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = TcpStream::connect(addr).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[test]
    fn raw_header_helpers() {
        let headers = b"POST / HTTP/1.1\r\ncontent-LENGTH: 42\r\nExpect: 100-Continue";

        assert_eq!(content_length(headers), 42);
        assert!(expects_continue(headers));
        assert_eq!(content_length(b"GET / HTTP/1.1\r\n"), 0);
        assert!(!expects_continue(b"GET / HTTP/1.1\r\n"));
    }

    #[tokio::test]
    async fn reads_a_body_bigger_than_one_read() {
        let (mut client, mut server) = socket_pair().await;
        let body = vec![b'a'; 10_000];
        let mut request = format!(
            "POST /media/upload HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .into_bytes();
        request.extend_from_slice(&body);
        client.write_all(&request).await.unwrap();

        let mut pending = Vec::new();
        let raw = match read_request(&mut server, &mut pending).await {
            Ok(raw) => raw,
            Err(_) => panic!("read_request failed"),
        };

        assert_eq!(Request::parse(&raw).unwrap().body.len(), 10_000);
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn reads_a_request_split_across_writes() {
        let (mut client, mut server) = socket_pair().await;

        tokio::spawn(async move {
            for part in [
                "POST / HTTP/1.1\r\nCont",
                "ent-Length: 10\r\n\r\n12345",
                "67890",
            ] {
                client.write_all(part.as_bytes()).await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        });

        let mut pending = Vec::new();
        let raw = match read_request(&mut server, &mut pending).await {
            Ok(raw) => raw,
            Err(_) => panic!("read_request failed"),
        };

        assert_eq!(Request::parse(&raw).unwrap().body, b"1234567890");
    }

    #[tokio::test]
    async fn keeps_pipelined_bytes_for_the_next_request() {
        let (mut client, mut server) = socket_pair().await;
        client
            .write_all(b"POST /a HTTP/1.1\r\nContent-Length: 3\r\n\r\nabcGET /b HTTP/1.1\r\n\r\n")
            .await
            .unwrap();

        let mut pending = Vec::new();
        let first = match read_request(&mut server, &mut pending).await {
            Ok(raw) => Request::parse(&raw).unwrap(),
            Err(_) => panic!("first read_request failed"),
        };
        let second = match read_request(&mut server, &mut pending).await {
            Ok(raw) => Request::parse(&raw).unwrap(),
            Err(_) => panic!("second read_request failed"),
        };

        assert_eq!(
            (first.path.as_str(), first.body.as_slice()),
            ("/a", &b"abc"[..])
        );
        assert_eq!(second.path, "/b");
    }

    #[tokio::test]
    async fn rejects_oversized_requests() {
        let (mut client, mut server) = socket_pair().await;
        let request = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_SIZE + 1
        );
        client.write_all(request.as_bytes()).await.unwrap();
        let mut pending = Vec::new();
        assert!(matches!(
            read_request(&mut server, &mut pending).await,
            Err(ReadError::BodyTooLarge)
        ));

        let (mut client, mut server) = socket_pair().await;
        let big_header = format!(
            "GET / HTTP/1.1\r\nX-Big: {}\r\n",
            "a".repeat(MAX_HEADER_SIZE * 2)
        );
        client.write_all(big_header.as_bytes()).await.unwrap();
        let mut pending = Vec::new();
        assert!(matches!(
            read_request(&mut server, &mut pending).await,
            Err(ReadError::HeadersTooLarge)
        ));
    }

    #[tokio::test]
    async fn reports_closed_connection() {
        let (client, mut server) = socket_pair().await;
        drop(client);

        let mut pending = Vec::new();
        assert!(matches!(
            read_request(&mut server, &mut pending).await,
            Err(ReadError::Closed)
        ));
    }

    #[test]
    fn forge_routes_answer() {
        let router = build_router();
        let send = |raw: &[u8]| router.handle(&Request::parse(raw).unwrap());

        assert_eq!(send(b"GET / HTTP/1.1\r\n\r\n").status as u16, 200);
        assert_eq!(
            send(b"GET /health HTTP/1.1\r\n\r\n").body,
            br#"{"status": "ok"}"#
        );
        assert_eq!(
            send(b"GET /media/abc HTTP/1.1\r\n\r\n").body,
            br#"{"id": "abc", "status": "found"}"#
        );
        assert_eq!(
            send(b"POST /media/upload HTTP/1.1\r\n\r\nhello").body,
            br#"{"received": "5 bytes"}"#
        );
        assert_eq!(
            send(b"DELETE /media/abc HTTP/1.1\r\n\r\n").body,
            br#"{"deleted": "abc"}"#
        );
        assert_eq!(send(b"GET /nope HTTP/1.1\r\n\r\n").status as u16, 404);
    }
}
