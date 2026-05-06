mod handler;
mod request;
mod response;
mod router;

use handler::{FnHandler, Handler};
use request::Request;
use response::Response;
use router::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::collections::HashMap;

use std::thread;
use std::time::Duration;

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

// Handles one connection from start to finish
// This runs as an independent Tokio task
// Multiple of these run simultaneously
async fn handle_connection(mut socket: TcpStream, router: Arc<Router>) {
    let mut buffer = vec![0u8; 4096];
    let bytes_read = match socket.read(&mut buffer).await {
        Ok(0) => {
            //0 bytes means the client closed the connection
            return;
        }
        Ok(n) => n,
        Err(e) => {
            eprintln!("Read error: {}", e);
            return;
        }
    };

    let response = match Request::parse(&buffer[..bytes_read]) {
        Ok(req) => {
            // Simulate slow work
            // tokio::time::sleep is async — it yields to the runtime
            // while sleeping, other tasks can run
            // this is fundamentally different from thread::sleep
            // which blocks the entire OS thread
            // tokio::time::sleep(Duration::from_secs(3)).await;

            println!("{:?} {} — done", req.method, req.path);
            router.handle(&req)
        }
        Err(e) => {
            eprintln!("Parse error: {}", e);
            Response::bad_request("Could not parse request")
        }
    };

    if let Err(e) = socket.write_all(&response.to_bytes()).await {
        eprintln!("Write error: {}", e);
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