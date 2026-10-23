//! Framework-free request and response types. The desktop shell translates its
//! custom-protocol requests into [`Request`] and the [`Response`] back, so the
//! API never opens a socket.

use std::collections::HashMap;

use serde::Serialize;

#[derive(Debug, Clone, Default)]
pub struct Request {
    /// Upper-case method (`GET`, `POST`, ...).
    pub method: String,
    /// Path without the query string, e.g. `/api/cargues/CargueA/informe`.
    pub path: String,
    /// Decoded query pairs in order of appearance (a key may repeat).
    pub query: Vec<(String, String)>,
    /// Lower-case header names.
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    /// Path captures filled by the router, in pattern order (`{id}` -> `params[0]`).
    pub params: Vec<String>,
}

impl Request {
    pub fn new(method: &str, path: &str) -> Self {
        Self {
            method: method.to_ascii_uppercase(),
            path: path.to_string(),
            ..Self::default()
        }
    }

    pub fn with_query(mut self, key: &str, value: &str) -> Self {
        self.query.push((key.to_string(), value.to_string()));
        self
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .insert(name.to_ascii_lowercase(), value.to_string());
        self
    }

    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// First value of the query key, if present.
    pub fn query_one(&self, key: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// Every value of the query key, in order.
    pub fn query_all(&self, key: &str) -> Vec<&str> {
        self.query
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    /// The i-th path capture (empty when missing).
    pub fn param(&self, index: usize) -> &str {
        self.params.get(index).map(String::as_str).unwrap_or("")
    }
}

#[derive(Debug, Clone)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// 200 with a JSON body (`Content-Type: application/json; charset=utf-8`,
    /// trailing newline, like Go's encoder).
    pub fn json<T: Serialize>(value: &T) -> Self {
        Self::json_status(200, value)
    }

    pub fn json_status<T: Serialize>(status: u16, value: &T) -> Self {
        let mut body = serde_json::to_vec(value).unwrap_or_else(|_| b"null".to_vec());
        body.push(b'\n');
        Self {
            status,
            headers: vec![(
                "Content-Type".to_string(),
                "application/json; charset=utf-8".to_string(),
            )],
            body,
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}
