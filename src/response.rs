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
#[derive(Debug, Clone, Copy)]
pub enum StatusCode {
    Ok = 200,
    Created = 201,
    NoContent = 204,
    BadRequest = 400,
    NotFound = 404,
    MethodNotAllowed = 405,
    InternalServerError = 500,
}

impl StatusCode {
    // Every status code has a standard reason phrase
    // The browser doesn't use this but it's part of the HTTP spec
    pub fn reason(&self) -> &str {
        match self {
            StatusCode::Ok                  => "OK",
            StatusCode::Created             => "Created",
            StatusCode::NoContent           => "No Content",
            StatusCode::BadRequest          => "Bad Request",
            StatusCode::NotFound            => "Not Found",
            StatusCode::MethodNotAllowed    => "Method Not Allowed",
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
        Self::new(StatusCode::NotFound)
            .body("<h1>404 Not Found</h1>", "text/html")
    }

    pub fn bad_request(msg: &str) -> Self {
        Self::new(StatusCode::BadRequest)
            .body(msg, "text/plain")
    }

    pub fn internal_error() -> Self {
        Self::new(StatusCode::InternalServerError)
            .body("Internal Server Error", "text/plain")
    }

    // Builder methods — set body and content-type together
    // Returns Self so you can chain: Response::ok().body("hello", "text/plain")
    pub fn body(mut self, content: &str, content_type: &str) -> Self {
        self.body = content.as_bytes().to_vec();
        self.headers.insert(
            "Content-Type".to_string(),
            content_type.to_string(),
        );
        self
    }

    // For binary responses — images, video bytes, downloads
    pub fn bytes(mut self, data: Vec<u8>, content_type: &str) -> Self {
        self.body = data;
        self.headers.insert(
            "Content-Type".to_string(),
            content_type.to_string(),
        );
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
        let content_length = format!(
            "Content-Length: {}\r\n",
            self.body.len()
        );
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