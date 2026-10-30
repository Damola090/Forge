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

        // Fractional milliseconds — most handlers finish in well
        // under 1ms, which as_millis() would round down to 0
        let ms = duration.as_secs_f64() * 1000.0;

        println!("  took {:.3}ms", ms);

        // Add timing as a response header
        // This lets API clients see how long the server took
        response.header(
            "X-Response-Time",
            &format!("{:.3}ms", ms),
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
        // Start at the outermost middleware
        // Each one calls next.handle() to run the rest of the chain
        Next {
            middlewares: &self.middlewares,
            inner: self.inner.as_ref(),
        }
        .handle(req, params)
    }
}

// The rest of the chain, from one middleware's point of view
// Calling handle() runs the next middleware — which gets its own Next
// for the middleware after it — until none are left and the real
// handler runs. Each middleware wraps everything inside it:
//
//   Logging → Timing → RequestId → handler → RequestId → Timing → Logging
//
// This is how Timing measures the handler and Logging sees the request
// before anything else runs
struct Next<'a> {
    middlewares: &'a [Box<dyn Middleware + Send + Sync>],
    inner: &'a dyn Handler,
}

impl Handler for Next<'_> {
    fn handle(&self, req: &Request, params: &HashMap<String, String>) -> Response {
        match self.middlewares.split_first() {
            // Run this middleware, handing it the remainder of the chain
            Some((middleware, rest)) => middleware.handle(
                req,
                params,
                &Next {
                    middlewares: rest,
                    inner: self.inner,
                },
            ),
            // No middleware left — run the actual handler
            None => self.inner.handle(req, params),
        }
    }
}
