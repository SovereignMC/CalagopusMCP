use serde_json::{Map, Value, json};

pub const SUPPORTED_VERSIONS: [&str; 4] = [
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];
pub const LATEST_VERSION: &str = "2025-11-25";

pub struct Request {
    pub id: Value,
    pub method: String,
    pub params: Map<String, Value>,
}

pub enum Prepared {
    Request(Request),
    /// A message without an `id`: acknowledged with an empty `202`.
    Notification,
    /// Rejected before dispatch. `status` is `400` for transport-level failures and `200` for
    /// valid JSON that is not a well-formed JSON-RPC request.
    Invalid {
        status: u16,
        error: Value,
    },
}

pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message.into(),
        },
    })
}

pub fn prepare(body: &[u8]) -> Prepared {
    let value: Value = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(err) => {
            return Prepared::Invalid {
                status: 400,
                error: error_response(Value::Null, -32700, format!("parse error: {err}")),
            };
        }
    };

    let Value::Object(mut message) = value else {
        return Prepared::Invalid {
            status: 400,
            error: error_response(
                Value::Null,
                -32600,
                "invalid request: the body must be a single json object, batches are not supported",
            ),
        };
    };

    let id = match message.remove("id") {
        None => None,
        Some(id @ (Value::String(_) | Value::Number(_))) => Some(id),
        Some(_) => {
            return Prepared::Invalid {
                status: 200,
                error: error_response(
                    Value::Null,
                    -32600,
                    "invalid request: `id` must be a string or a number",
                ),
            };
        }
    };

    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Prepared::Invalid {
            status: 200,
            error: error_response(
                id.unwrap_or(Value::Null),
                -32600,
                "invalid request: `jsonrpc` must be \"2.0\"",
            ),
        };
    }

    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Prepared::Invalid {
            status: 200,
            error: error_response(
                id.unwrap_or(Value::Null),
                -32600,
                "invalid request: `method` must be a string",
            ),
        };
    };
    let method = method.to_string();

    let params = match message.remove("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => {
            return Prepared::Invalid {
                status: 200,
                error: error_response(
                    id.unwrap_or(Value::Null),
                    -32602,
                    "invalid params: `params` must be an object",
                ),
            };
        }
    };

    match id {
        Some(id) => Prepared::Request(Request { id, method, params }),
        None => Prepared::Notification,
    }
}
