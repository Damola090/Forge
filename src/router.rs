use crate::handler::Handler;
use crate::request::{Method, Request};
use crate::response::Response;
use std::collections::HashMap;

// A single registered route
struct Route {
    method: Method,
    // Pattern like "/media/:id/transcode"
    // Segments starting with : are parameters
    pattern: String,
    // Box<dyn Handler> — heap allocated, runtime dispatched
    // This is why we need Box — each handler is a different
    // concrete type with a different size
    // Box makes them all pointer-sized so Vec can hold them
    handler: Box<dyn Handler + Send + Sync>,
}

pub struct Router {
    routes: Vec<Route>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
        }
    }

    // Register a GET route
    pub fn get(&mut self, pattern: &str, handler: Box<dyn Handler + Send + Sync>) {
        self.routes.push(Route {
            method: Method::Get,
            pattern: pattern.to_string(),
            handler,
        })
    }

    // Register a POST route
    pub fn post(&mut self, pattern: &str, handler: Box<dyn Handler + Send + Sync>) {
        self.routes.push(Route {
            method: Method::Post,
            pattern: pattern.to_string(),
            handler,
        });
    }

    // Register a DELETE route
    pub fn delete(&mut self, pattern: &str, handler: Box<dyn Handler + Send + Sync>) {
        self.routes.push(Route {
            method: Method::Delete,
            pattern: pattern.to_string(),
            handler,
        });
    }

    // Match an incoming request to a registered route
    // Returns the response — either from the matched handler or 404
    pub fn handle(&self, req: &Request) -> Response {
        for route in &self.routes {
            // Method must match first
            if route.method != req.method {
                continue;
            }

            //Try to match the path pattern
            // If it matches, params contains extracted path parameters
            if let Some(params) = match_pattern(&route.pattern, &req.path) {
                return route.handler.handle(req, &params);
            }
        }

        // No Route matched - 404
        Response::not_found()
    }
}

// Pattern matching — the core of routing
// "/media/:id" matches "/media/abc123" and extracts id="abc123"
// "/health" matches "/health" exactly

fn match_pattern(pattern: &str, path: &str) -> Option<HashMap<String, String>> {
    let pattern_parts: Vec<&str> = pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    let path_parts: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    // Different number of segments — no match
    // "/media/:id" has 2 segments, "/media/abc/extra" has 3 — no match
    if pattern_parts.len() != path_parts.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (pattern_seg, path_seg) in pattern_parts.iter().zip(path_parts.iter()) {
        if pattern_seg.starts_with(':') {
            // This segment is a parameter — extract the name and value
            // ":id" with "abc123" → params["id"] = "abc123"
            let param_name = &pattern_seg[1..]; // strip the leading :
            params.insert(param_name.to_string(), path_seg.to_string());
        } else if pattern_seg != path_seg {
            // Static segment that doesn't match — no match
            return None;
        }
        // Static segment that matches — continue
    }

    Some(params)

}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::FnHandler;

    fn text(response: &Response) -> String {
        String::from_utf8_lossy(&response.to_bytes()).to_string()
    }

    fn send(router: &Router, raw: &[u8]) -> String {
        text(&router.handle(&Request::parse(raw).unwrap()))
    }

    fn test_router() -> Router {
        let mut router = Router::new();
        router.get("/health", Box::new(FnHandler::new(|_req, _params| {
            Response::ok().body("ok", "text/plain")
        })));
        router.get("/media/:id", Box::new(FnHandler::new(|_req, params| {
            Response::ok().body(&format!("get {}", params["id"]), "text/plain")
        })));
        router.delete("/media/:id", Box::new(FnHandler::new(|_req, params| {
            Response::ok().body(&format!("delete {}", params["id"]), "text/plain")
        })));
        router
    }

    #[test]
    fn match_pattern_static_and_params() {
        assert_eq!(match_pattern("/health", "/health"), Some(HashMap::new()));
        assert_eq!(match_pattern("/health", "/status"), None);

        let params = match_pattern("/media/:id/transcode", "/media/abc/transcode").unwrap();
        assert_eq!(params["id"], "abc");

        // Different number of segments never matches
        assert_eq!(match_pattern("/media/:id", "/media/abc/extra"), None);
        assert_eq!(match_pattern("/media/:id", "/media"), None);
    }

    #[test]
    fn routes_by_method_and_path() {
        let router = test_router();

        assert!(send(&router, b"GET /health HTTP/1.1\r\n\r\n").ends_with("ok"));
        assert!(send(&router, b"GET /media/42 HTTP/1.1\r\n\r\n").ends_with("get 42"));
        assert!(send(&router, b"DELETE /media/42 HTTP/1.1\r\n\r\n").ends_with("delete 42"));
    }

    #[test]
    fn unmatched_requests_get_404() {
        let router = test_router();

        assert!(send(&router, b"GET /nope HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
        // Path exists, method doesn't — currently 404 too
        assert!(send(&router, b"POST /health HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 404"));
    }
}
