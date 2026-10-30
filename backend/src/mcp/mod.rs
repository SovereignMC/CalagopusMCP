use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::Request,
    http::{HeaderValue, Method, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use utoipa_axum::router::OpenApiRouter;

use crate::{openapi::Index, panel::Caller};

pub mod dispatch;
pub mod protocol;

pub const SERVER_NAME: &str = "calagopus-mcp";
pub const MAX_BODY: usize = 64 * 1024 * 1024;
pub const MAX_RESPONSE: usize = 16 * 1024 * 1024;
pub const MAX_TEXT: usize = 100_000;
pub const MAX_UPLOAD: usize = 32 * 1024 * 1024;
pub const MAX_DOWNLOAD: usize = 4 * 1024 * 1024;

#[derive(Clone)]
struct McpState {
    state: shared::State,
    index: Arc<tokio::sync::OnceCell<Arc<Index>>>,
}

pub fn router(state: &shared::State) -> OpenApiRouter<shared::State> {
    OpenApiRouter::new()
        .route("/mcp", axum::routing::any(handle))
        .with_state(state.clone())
        .layer(axum::Extension(McpState {
            state: state.clone(),
            index: Arc::new(tokio::sync::OnceCell::new()),
        }))
        .route_layer(axum::middleware::from_fn(require_api_key))
}

async fn require_api_key(
    axum::Extension(auth): axum::Extension<Arc<shared::models::user::AuthMethod>>,
    req: Request,
    next: Next,
) -> Response {
    if !matches!(*auth, shared::models::user::AuthMethod::ApiKey(_)) {
        return error_response(
            StatusCode::FORBIDDEN,
            "connect with a panel API key as \"Authorization: Bearer <key>\"",
        );
    }

    next.run(req).await
}

async fn handle(axum::Extension(mcp): axum::Extension<McpState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();

    let body = match axum::body::to_bytes(body, MAX_BODY).await {
        Ok(body) => body,
        Err(_) => {
            return error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request body larger than 64 MiB",
            );
        }
    };

    let caller = match Caller::from_parts(&parts) {
        Ok(caller) => caller,
        Err(err) => return error_response(StatusCode::INTERNAL_SERVER_ERROR, err),
    };

    let panel = crate::panel::RealPanel::new(mcp.state.clone(), caller, mcp.index.clone());

    respond(parts.method, body, &panel).await
}

/// Everything after the panel authentication context exists. Takes the panel by trait so the
/// whole HTTP surface — method gate, JSON-RPC framing and response shape — can be exercised
/// without a database or a router.
pub(crate) async fn respond(method: Method, body: Bytes, panel: &dyn crate::panel::PanelApi) -> Response {
    if method != Method::POST {
        return (
            StatusCode::METHOD_NOT_ALLOWED,
            [(header::ALLOW, HeaderValue::from_static("POST"))],
            "this endpoint accepts MCP messages with POST only",
        )
            .into_response();
    }

    match protocol::prepare(&body) {
        protocol::Prepared::Notification => StatusCode::ACCEPTED.into_response(),
        protocol::Prepared::Invalid { status, error } => json_response(status, error),
        protocol::Prepared::Request(request) => {
            json_response(StatusCode::OK.as_u16(), dispatch::dispatch(panel, request).await)
        }
    }
}

fn json_response(status: u16, value: Value) -> Response {
    let mut response = axum::Json(value).into_response();
    *response.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::OK);
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));

    response
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response {
    json_response(status.as_u16(), json!({ "error": message.into() }))
}
