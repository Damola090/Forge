use crate::request::Request;
use crate::response::Response;
use std::collections::HashMap;

// A Handler is anything that can process a request
// This is a trait — not a concrete type
// Any struct that implements this can be used as a route handler
pub trait Handler {
    fn handle(&self, req: &Request, params: &HashMap<String, String>) -> Response;
}

// This is the key — it lets you pass a plain function as a handler
// instead of always needing a struct
// FnHandler wraps a closure or function pointer
pub struct FnHandler<F>
where
    F: Fn(&Request, &HashMap<String, String>) -> Response,
{
    func: F,
}

impl<F> FnHandler<F>
where 
    F: Fn(&Request, &HashMap<String, String>) -> Response,
{
    pub fn new(func: F) -> Self {
        Self { func }
    }
}

// Implement Handler for FnHandler
// Now any function with the right signature IS a Handler
impl<F> Handler for FnHandler<F>
where 
    F: Fn(&Request, &HashMap<String, String>) -> Response,
{
    fn handle(&self, req: &Request, params: &HashMap<String, String>) -> Response {
        (self.func)(req, params)
    }
}