mod handler;
mod request;
mod response;
mod router;
mod middleware;

use handler::{FnHandler, Handler};
use request::Request;
use response::{Response, StatusCode};
use router::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use middleware::{LoggingMiddleware, MiddlewareChain, RequestIdMiddleware, TimingMiddleware};
use std::sync::Arc;
use std::collections::HashMap;

use std::thread;
use std::time::Duration;

fn build_router() -> Router {
    let mut router = Router::new();

    // Wrap each handler in a middleware chain
    // Every route gets logging, timing, and request ID automatically
    // The handler itself knows nothing about any of this

    router.get("/", Box::new(
        MiddlewareChain::new(Box::new(FnHandler::new(|_req, _params| {
            Response::ok().body("<h1>Welcome to Forge</h1>", "text/html")
        })))
        .with(Box::new(LoggingMiddleware))
        .with(Box::new(TimingMiddleware))
        .with(Box::new(RequestIdMiddleware))
    ));

    router.get("/health", Box::new(
        MiddlewareChain::new(Box::new(FnHandler::new(|_req, _params| {
            Response::ok().body(r#"{"status": "ok"}"#, "application/json")
        })))
        .with(Box::new(LoggingMiddleware))
        .with(Box::new(TimingMiddleware))
        .with(Box::new(RequestIdMiddleware))
    ));

    router.get("/media/:id", Box::new(
        MiddlewareChain::new(Box::new(FnHandler::new(|_req, params| {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("unknown");
            let body = format!(r#"{{"id": "{}", "status": "found"}}"#, id);
            Response::ok().body(&body, "application/json")
        })))
        .with(Box::new(LoggingMiddleware))
        .with(Box::new(TimingMiddleware))
        .with(Box::new(RequestIdMiddleware))
    ));

    router.post("/media/upload", Box::new(
        MiddlewareChain::new(Box::new(FnHandler::new(|req, _params| {
            let size = req.body.len();
            let body = format!(r#"{{"received": "{} bytes"}}"#, size);
            Response::ok().body(&body, "application/json")
        })))
        .with(Box::new(LoggingMiddleware))
        .with(Box::new(TimingMiddleware))
        .with(Box::new(RequestIdMiddleware))
    ));

    router.delete("/media/:id", Box::new(
        MiddlewareChain::new(Box::new(FnHandler::new(|_req, params| {
            let id = params.get("id").map(|s| s.as_str()).unwrap_or("unknown");
            let body = format!(r#"{{"deleted": "{}"}}"#, id);
            Response::ok().body(&body, "application/json")
        })))
        .with(Box::new(LoggingMiddleware))
        .with(Box::new(TimingMiddleware))
        .with(Box::new(RequestIdMiddleware))
    ));

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
                let wants_close = req.header("connection")
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
                let response = Response::bad_request("Could not parse request")
                    .header("Connection", "close");
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
        let (socket, addr) = match listener.accept().await{
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