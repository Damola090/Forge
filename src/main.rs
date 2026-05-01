mod request;
mod response;

use tokio::net::TcpListener;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use request::Request;
use response::{Response, StatusCode};

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("Forge listening on port 8080");

    loop {
        let (mut socket, addr) = listener.accept().await.unwrap();
        println!("Connection from {}", addr);

        let mut buffer = vec![0u8; 4096];
        let bytes_read = socket.read(&mut buffer).await.unwrap();

        let response = match Request::parse(&buffer[..bytes_read]) {
            Ok(req) => {
                println!("{:?} {}", req.method, req.path);

                // Pattern match on the path — this is manual routing for now
                // Phase 4 replaces this with a proper Router
                match req.path.as_str() {
                    "/" => Response::ok()
                        .body("<h1>Welcome to Forge</h1>", "text/html"),

                    "/health" => Response::ok()
                        .body(r#"{"status": "ok"}"#, "application/json"),

                    "/about" => Response::ok()
                        .body("<h1>Forge HTTP Server</h1><p>Built from scratch in Rust</p>", "text/html"),

                    // Catch-all — anything else is a 404
                    _ => Response::not_found(),
                }
            }

            // Parse failed — send a 400
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