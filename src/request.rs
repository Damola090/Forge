use std::collections::HashMap;

// This is what a raw HTTP request becomes after parsing
// Every field is strongly typed — method is not a String, it's a Method enum
#[derive(Debug)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub query_string: Option<String>,
}

// Method is an enum — not a string
// The compiler forces you to handle every possible method
#[derive(Debug, PartialEq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

// Represents everything that can go wrong during parsing
#[derive(Debug)]
pub enum ParseError {
    EmptyRequest,
    InvalidRequestLine,
    InvalidMethod(String),
    MissingPath,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::EmptyRequest        => write!(f, "Empty request"),
            ParseError::InvalidRequestLine  => write!(f, "Invalid request line"),
            ParseError::InvalidMethod(m)    => write!(f, "Unknown method: {}", m),
            ParseError::MissingPath         => write!(f, "Missing path"),
        }
    }
}

// --- Raw Request ---
// GET / HTTP/1.1
// Host: localhost:8080
// Sec-Fetch-Dest: document
// User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.2 Safari/605.1.15
// Upgrade-Insecure-Requests: 1
// Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8
// Sec-Fetch-Site: none
// Sec-Fetch-Mode: navigate
// Accept-Language: en-US,en;q=0.9
// Priority: u=0, i
// Accept-Encoding: gzip, deflate
// Cookie: sidebar_state=true
// Connection: keep-alive

impl Request {
    // Takes raw bytes from the socket and returns a structured Request
    // This is the function that makes HTTP readable
    pub fn parse(raw: &[u8]) -> Result<Self, ParseError> {

        if raw.is_empty() {
            return Err(ParseError::EmptyRequest);
        }

        // Split headers from body on the blank line
        // Done on the raw bytes, not on text — the body can be binary
        // (images, video) and converting it to a string would corrupt it
        let (header_bytes, body) = match header_end(raw) {
            Some((headers_end, body_start)) => (&raw[..headers_end], &raw[body_start..]),
            None => (raw, &[][..]),
        };

        // Convert the header bytes to a string so we can work with text
        // from_utf8_lossy replaces invalid UTF-8 with a placeholder
        // instead of panicking — important for real network data
        let header_section = String::from_utf8_lossy(header_bytes);

        // Split the header section into individual lines
        let mut lines = header_section.lines();

        // First line is always the request line: GET /path HTTP/1.1
        let request_line = lines
            .next()
            .ok_or(ParseError::InvalidRequestLine)?;

        // Split the request line into its three parts
        let mut parts = request_line.split_whitespace();

        let method_str = parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)?;

        let raw_path = parts
            .next()
            .ok_or(ParseError::MissingPath)?;

        // _version is the HTTP version — we read it but don't use it yet
        let _version = parts.next();

        // Parse the method string into our Method enum
        let method = Method::parse(method_str)?;

        // Separate path from query string
        // /search?q=rust&page=2  →  path="/search", query="q=rust&page=2"
        let (path, query_string) = match raw_path.split_once('?') {
            Some((p, q)) => (p.to_string(), Some(q.to_string())),
            None         => (raw_path.to_string(), None),
        };

        // Parse remaining lines as headers
        // Each header looks like:  Header-Name: Header-Value
        let mut headers = HashMap::new();

        for line in lines {
            // split_once splits on the FIRST occurrence only
            // important because header values can contain colons
            // e.g. Date: Mon, 01 Jan 2026 00:00:00 GMT
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(
                    key.trim().to_lowercase(),   // normalize to lowercase
                    value.trim().to_string(),
                );
            }
        }

        Ok(Request {
            method,
            path,
            headers,
            body: body.to_vec(),
            query_string,
        })
    }

    // Convenience — get a header value by name
    pub fn header(&self, name: &str) -> Option<&String> {
        self.headers.get(&name.to_lowercase())
    }

    // Convenience — get content length if present
    pub fn content_length(&self) -> usize {
        self.header("content-length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    }
}

// Find where the headers end
// HTTP spec says headers end at \r\n\r\n
// Some clients send \n\n instead — be lenient, accept both
// Returns (end of the header section, start of the body)
pub fn header_end(raw: &[u8]) -> Option<(usize, usize)> {
    if let Some(pos) = find(raw, b"\r\n\r\n") {
        return Some((pos, pos + 4));
    }
    find(raw, b"\n\n").map(|pos| (pos, pos + 2))
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

impl Method {
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        match s.to_uppercase().as_str() {
            "GET"     => Ok(Method::Get),
            "POST"    => Ok(Method::Post),
            "PUT"     => Ok(Method::Put),
            "DELETE"  => Ok(Method::Delete),
            "PATCH"   => Ok(Method::Patch),
            "HEAD"    => Ok(Method::Head),
            "OPTIONS" => Ok(Method::Options),
            other     => Err(ParseError::InvalidMethod(other.to_string())),
        }
    }
}


