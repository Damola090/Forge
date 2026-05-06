mod handler;
mod request;
mod response;
mod router;
mod middleware;

use handler::{FnHandler, Handler};
use request::Request;
use response::Response;
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

// Handles one connection from start to finish
// This runs as an independent Tokio task
// Multiple of these run simultaneously
async fn handle_connection(mut socket: TcpStream, router: Arc<Router>) {
    let mut buffer = vec![0u8; 4096];


    // Loop — handle multiple requests on the same connection
    // This is keep-alive — the connection stays open
    loop {

        let bytes_read = match socket.read(&mut buffer).await {
            Ok(0) => {
                // Client closed the connection cleanly
                    // 0 bytes = EOF = they're done
                    println!("Client disconnected");
                return;
            }
            Ok(n) => n,
            Err(e) => {
                eprintln!("Read error: {}", e);
                return;
            }
        };

        let (response, should_close) = match Request::parse(&buffer[..bytes_read]) {
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