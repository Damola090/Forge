use std::collections::HashMap;

// Every HTTP response has these three parts
#[derive(Debug, Clone)]
pub struct Response {
    pub status: StatusCode,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

// Status codes as a typed enum
// Not magic numbers — named variants the compiler checks
// Some codes aren't used by any handler yet (Created, NoContent,
// MethodNotAllowed, InternalServerError) — they're the vocabulary
// the MediaLab handlers will need, so don't warn about them
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum StatusCode {
    Ok = 200,
    Created = 201,
    NoContent = 204,
    BadRequest = 400,
    NotFound = 404,
    MethodNotAllowed = 405,
    PayloadTooLarge = 413,
    RequestHeaderFieldsTooLarge = 431,
    InternalServerError = 500,
}

impl StatusCode {
    // Every status code has a standard reason phrase
    // The browser doesn't use this but it's part of the HTTP spec
    pub fn reason(&self) -> &str {
        match self {
            StatusCode::Ok => "OK",
            StatusCode::Created => "Created",
            StatusCode::NoContent => "No Content",
            StatusCode::BadRequest => "Bad Request",
            StatusCode::NotFound => "Not Found",
            StatusCode::MethodNotAllowed => "Method Not Allowed",
            StatusCode::PayloadTooLarge => "Payload Too Large",
            StatusCode::RequestHeaderFieldsTooLarge => "Request Header Fields Too Large",
            StatusCode::InternalServerError => "Internal Server Error",
        }
    }
}

impl Response {
    // Start with an empty response — headers added separately
    pub fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    // Convenience constructors for common responses
    pub fn ok() -> Self {
        Self::new(StatusCode::Ok)
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NotFound).body("<h1>404 Not Found</h1>", "text/html")
    }

    pub fn bad_request(msg: &str) -> Self {
        Self::new(StatusCode::BadRequest).body(msg, "text/plain")
    }

    // Not used yet — for handlers that can fail (Layer 8)
    #[allow(dead_code)]
    pub fn internal_error() -> Self {
        Self::new(StatusCode::InternalServerError).body("Internal Server Error", "text/plain")
    }

    // Builder methods — set body and content-type together
    // Returns Self so you can chain: Response::ok().body("hello", "text/plain")
    pub fn body(mut self, content: &str, content_type: &str) -> Self {
        self.body = content.as_bytes().to_vec();
        self.headers
            .insert("Content-Type".to_string(), content_type.to_string());
        self
    }

    // For binary responses — images, video bytes, downloads
    // Not used yet — MediaLab will serve media through it (Layer 8)
    #[allow(dead_code)]
    pub fn bytes(mut self, data: Vec<u8>, content_type: &str) -> Self {
        self.body = data;
        self.headers
            .insert("Content-Type".to_string(), content_type.to_string());
        self
    }

    // Add any header
    pub fn header(mut self, key: &str, value: &str) -> Self {
        self.headers.insert(key.to_string(), value.to_string());
        self
    }

    // Serialize the entire response into raw bytes
    // This is what gets written to the TCP socket
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut output = Vec::new();

        // Status line: HTTP/1.1 200 OK\r\n
        let status_line = format!(
            "HTTP/1.1 {} {}\r\n",
            self.status as u16,
            self.status.reason()
        );
        output.extend_from_slice(status_line.as_bytes());

        // Content-Length is mandatory — without it the browser
        // doesn't know when the response body ends and hangs forever
        // This is why Safari kept retrying — no response at all
        let content_length = format!("Content-Length: {}\r\n", self.body.len());
        output.extend_from_slice(content_length.as_bytes());

        // Write all other headers
        for (key, value) in &self.headers {
            let header_line = format!("{}: {}\r\n", key, value);
            output.extend_from_slice(header_line.as_bytes());
        }

        // Blank line separates headers from body
        // This is the \r\n\r\n your parser looked for
        output.extend_from_slice(b"\r\n");

        // Body bytes
        output.extend_from_slice(&self.body);

        output
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn text(response: &Response) -> String {
        String::from_utf8_lossy(&response.to_bytes()).to_string()
    }

    #[test]
    fn serializes_status_line_headers_and_body() {
        let out = text(&Response::ok().body("hello", "text/plain"));

        assert!(out.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(out.contains("Content-Length: 5\r\n"));
        assert!(out.contains("Content-Type: text/plain\r\n"));
        // Blank line, then the body
        assert!(out.ends_with("\r\n\r\nhello"));
    }

    #[test]
    fn content_length_counts_bytes_not_characters() {
        // "→" is 3 bytes in UTF-8
        let out = text(&Response::ok().body("→", "text/plain"));
        assert!(out.contains("Content-Length: 3\r\n"));
    }

    #[test]
    fn shortcut_constructors() {
        assert!(text(&Response::not_found()).starts_with("HTTP/1.1 404 Not Found\r\n"));
        assert!(text(&Response::bad_request("nope")).ends_with("\r\n\r\nnope"));

        let out = text(&Response::internal_error());
        assert!(out.starts_with("HTTP/1.1 500 Internal Server Error\r\n"));
        assert!(out.ends_with("\r\n\r\nInternal Server Error"));
    }

    #[test]
    fn bytes_keeps_binary_body() {
        let data = vec![0u8, 255, 128, 10];
        let response = Response::ok().bytes(data.clone(), "image/png");

        assert_eq!(response.body, data);
        let out = text(&response);
        assert!(out.contains("Content-Type: image/png\r\n"));
        assert!(out.contains("Content-Length: 4\r\n"));
    }

    #[test]
    fn header_adds_custom_header() {
        let out = text(&Response::ok().header("X-Test", "1"));
        assert!(out.contains("X-Test: 1\r\n"));
    }
}
