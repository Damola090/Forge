mod handler;
mod request;
mod response;
mod router;

use handler::{FnHandler, Handler};
use request::Request;
use response::Response;
use router::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use std::collections::HashMap;

fn build_router() -> Router {
    let mut router = Router::new();

    // GET /
    router.get("/", Box::new(FnHandler::new(|_req, _params| {
        Response::ok()
             .body("<h1>Welcome to Forge</h1>", "text/html")
    })));

    // GET /health
    router.get("/health", Box::new(FnHandler::new(|_req, _params| {
        Response::ok()
            .body(r#"{"status": "ok"}"#, "application/json")
    })));

    // GET /media/:id
    // :id is a path parameter — extracted automatically by the router
    router.get("/media/:id", Box::new(FnHandler::new(|_req, params| {
        // params["id"] is whatever was in the URL
        let id = params.get("id").map(|s| s.as_str()).unwrap_or("unknown");
        let body = format!(r#"{{"id": "{}", "status": "found"}}"#, id);
        Response::ok().body(&body, "application/json")
    })));

    // POST /media/upload
    router.post("/media/upload", Box::new(FnHandler::new(|req, _params| {
        // req.body contains the uploaded bytes
        let size = req.body.len();
        let body = format!(r#"{{"received": {} bytes}}"#, size);
        Response::ok().body(&body, "application/json")
    })));

    // DELETE /media/:id
    router.delete("/media/:id", Box::new(FnHandler::new(|_req, params| {
        let id = params.get("id").map(|s| s.as_str()).unwrap_or("unknown");
        let body = format!(r#"{{"deleted": "{}"}}"#, id);
        Response::ok().body(&body, "application/json")
    })));

    router
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    let router = build_router();

    println!("Forge listening on port 8080");

    loop {
        let (mut socket, addr) = listener.accept().await.unwrap();
        println!("Connection from {}", addr);

        let mut buffer = vec![0u8; 4096];
        let bytes_read = socket.read(&mut buffer).await.unwrap();

        let response = match Request::parse(&buffer[..bytes_read]) {
            Ok(req) => {
                println!("{:?} {}", req.method, req.path);
                router.handle(&req)
            }
            Err(e) => {
                eprintln!("Parse error: {}", e);
                Response::bad_request("Could not parse request")
            }
        };

        // Serialize the response to bytes and write back to the socket
        // write_all guarantees every byte is sent — not just some of them
        let response_bytes = response.to_bytes();
        socket.write_all(&response_bytes).await.unwrap();

        println!(
            "Responded: {} {} → {}",
            match request::Request::parse(&buffer[..bytes_read]) {
                Ok(r) => format!("{:?}", r.method),
                Err(_) => "??".to_string(),
            },
            "path",
            response.status as u16
        );
    }
}