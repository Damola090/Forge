use tokio::net::TcpListener;
use tokio::io::AsyncReadExt;
use crate::request::Request;

mod request;

#[tokio::main]
async fn main() {
    // Bind to port 8080 on all interfaces
    // 0.0.0.0 means accept connections from anywhere
    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();

    println!("Forge listening on port 8080");

    loop {
        // accept() waits until a client connects
        // returns the socket and the client's address
        let (mut socket, addr) = listener.accept().await.unwrap();

        println!("Connection from {}", addr);
        println!("socket {:?}", socket);

        // Read raw bytes off the socket into a buffer
        let mut buffer = vec![0u8; 4096];
        let bytes_read = socket.read(&mut buffer).await.unwrap();

        // Parse the raw bytes into a structured Request
        match Request::parse(&buffer[..bytes_read]) {
            Ok(req) => {
                println!("Method:  {:?}", req.method);
                println!("Path:    {}", req.path);
                println!("Headers: {:?}", req.headers);
                println!("Query:   {:?}", req.query_string);
                println!("Body:    {} bytes", req.body.len());
            }
            Err(e) => {
                println!("Parse error: {}", e);
            }
        }
    }
}



