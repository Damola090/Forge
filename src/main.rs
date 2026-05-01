use tokio::net::TcpListener;
use tokio::io::AsyncReadExt;
use crate::request::Request

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

        // Print exactly what arrived — raw HTTP as the browser sent it
        let raw = String::from_utf8_lossy(&buffer[..bytes_read]);
        println!("--- Raw Request ---\n{}", raw);
    }
}
