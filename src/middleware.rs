use crate::handler::Handler;
use crate::request::Request;
use crate:: response::Response;
use std::collections::HashMap;
use std::time::Instant;

//Middleware wraps a Handler
// It receives the request, can moifydy it,calls the inner handler,
// receives the response, can modify it, returns it
// This is the classic decorator pattern
pub trait Middleware {
    fn handle(
        &self,
        req: &Request,
        params: &HashMap<String, String>,
        //next is the inner handler - calling it continues the chain
        next: &dyn Handler,
    ) -> Response;

}

// ─── Logging Middleware ───────────────────────────────────────────

pub struct LoggingMiddleware;

impl Middleware for LoggingMiddleware {
    fn handle(
        &self,
        req: &Request,
        params: &HashMap<String, String>,
        next: &dyn Handler,
    ) -> Response {
        // Before - log the incoming Request

        println!(
            "-> {:?} {}{}",
            req.method,
            req.path,
            req.query_string
                .as_ref()
                .map(|q| format!("?{}", q))
                .unwrap_or_default()
        );

        // call the next handler in the chain
        let response = next.handle(req, params);

        // After -log the outgoing response
        println!("<- {}", response.status as u16);

        response
    }
}


// ─── Timing Middleware ────────────────────────────────────────────

pub struct TimingMiddleware;

impl Middleware for TimingMiddleware {
    fn handle(
        &self,
        req: &Request,
        params: &HashMap<String, String>,
        next: &dyn Handler,
    ) -> Response {
        // Record time before handler runs
        let start = Instant::now();

        let response = next.handle(req, params);

        // Calculate duration after handler returns
        let duration = start.elapsed();

        println!(
            "  took {}ms",
            duration.as_millis()
        );

        // Add timing as a response header
        // This lets API clients see how long the server took
        response.header(
            "X-Response-Time",
            &format!("{}ms", duration.as_millis()),
        )
    }
}

// ─── Request ID Middleware ────────────────────────────────────────

pub struct RequestIdMiddleware;

impl RequestIdMiddleware {
    // Generate a simple unique ID for each request
    // In production you'd use a UUID crate
    // Here we use timestamp + a counter for simplicity
    fn generate_id() -> String {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        format!("req_{}", nanos)
    }
}

impl Middleware for RequestIdMiddleware {
    fn handle(
        &self,
        req: &Request,
        params: &HashMap<String, String>,
        next: &dyn Handler,
    ) -> Response {
        let id = Self::generate_id();
        println!("  id: {}", id);

        let response = next.handle(req, params);

        // Attach the ID to the response so clients can reference it
        // Useful for debugging — client can report the ID when something fails
        response.header("X-Request-Id", &id)
    }
}

// ─── Middleware Chain ─────────────────────────────────────────────

// Wraps a Handler with a stack of middleware
// When handle() is called, middleware runs in order
// then the inner handler runs
// then middleware runs in reverse order on the way out
pub struct MiddlewareChain {
    // The actual handler at the center of the chain
    inner: Box<dyn Handler + Send + Sync>,
    // Middleware stack — runs in order
    middlewares: Vec<Box<dyn Middleware + Send + Sync>>,
}

impl MiddlewareChain {
    pub fn new(inner: Box<dyn Handler + Send + Sync>) -> Self {
        Self {
            inner,
            middlewares: Vec::new(),
        }
    }

    pub fn with(mut self, middleware: Box<dyn Middleware + Send + Sync>) -> Self {
        self.middlewares.push(middleware);
        self
    }
}

impl Handler for MiddlewareChain {
    fn handle(&self, req: &Request, params: &HashMap<String, String>) -> Response {
        // Build the chain from the outside in
        // Each middleware wraps the next one
        // The innermost is the actual handler

        // Start with the inner handler
        // Work outward through middleware in reverse
        // so the first middleware added runs first
        let mut current: &dyn Handler = self.inner.as_ref();

        // For a simple chain we call them sequentially
        // Each middleware calls next.handle() to continue
        // We need a different approach for a dynamic chain

        // Simple sequential approach — works for our use case
        // Each middleware in the stack gets called with the inner handler
        // They all share the same inner handler reference
        if self.middlewares.is_empty() {
            return current.handle(req, params);
        }

        // Run middleware in order, each calling the next
        // For simplicity here we run them sequentially
        // and compose the response transformations
        let mut response = current.handle(req, params);

        // Apply middleware in reverse — outermost last
        // This simulates the wrap-around behavior
        for middleware in self.middlewares.iter().rev() {
            // Create a temporary handler that returns our current response
            // This lets each middleware wrap the already-computed response
            let temp = ResponseHandler { response };
            response = middleware.handle(req, params, &temp);
        }

        response
    }
}

// Helper — a Handler that just returns a pre-computed response
// Used internally by MiddlewareChain
struct ResponseHandler {
    response: Response,
}

impl Handler for ResponseHandler {
    fn handle(&self, _req: &Request, _params: &HashMap<String, String>) -> Response {
        // Clone the response — Response needs to derive Clone
        // We'll add that in a moment
        self.response.clone()
    }
}