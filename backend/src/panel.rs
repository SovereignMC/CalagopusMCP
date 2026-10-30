use std::{net::SocketAddr, sync::Arc};

use axum::http::{HeaderMap, Method, header};
use serde_json::{Map, Value};

use crate::openapi::Index;

const MAX_UPLOAD_RESPONSE: usize = 1024 * 1024;

/// Headers forwarded from the MCP request into the panel call. Authentication headers are
/// deliberately absent: the panel call is authenticated with the caller's own identity through
/// the router extensions, so forwarding a bearer token or session cookie would add nothing and
/// could only leak them into an unrelated request.
const FORWARDED_HEADERS: [&str; 6] = [
    "host",
    "user-agent",
    "x-forwarded-for",
    "x-real-ip",
    "x-forwarded-host",
    "x-forwarded-proto",
];

pub struct Reply {
    pub status: u16,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

pub struct Caller {
    pub user: shared::models::user::User,
    pub auth: shared::models::user::AuthMethod,
    pub addr: SocketAddr,
    pub headers: HeaderMap,
}

impl Caller {
    pub fn from_parts(parts: &axum::http::request::Parts) -> Result<Self, String> {
        let user = parts.extensions.get::<shared::models::user::User>();
        let auth = parts
            .extensions
            .get::<Arc<shared::models::user::AuthMethod>>();
        let addr = parts
            .extensions
            .get::<axum::extract::ConnectInfo<SocketAddr>>();

        match (user, auth, addr) {
            (Some(user), Some(auth), Some(addr)) => Ok(Self {
                user: user.clone(),
                auth: auth.as_ref().clone(),
                addr: addr.0,
                headers: parts.headers.clone(),
            }),
            _ => Err("missing panel authentication context".to_string()),
        }
    }
}

/// Everything the tools need from the panel. A trait rather than concrete calls so tool behaviour
/// can be exercised without a database, a router or a running panel.
#[async_trait::async_trait]
pub trait PanelApi: Send + Sync {
    async fn call(
        &self,
        method: Method,
        path: &str,
        query: Option<&Map<String, Value>>,
        body: Option<&Value>,
    ) -> Result<Reply, String>;

    async fn index(&self) -> Result<Arc<Index>, String>;

    async fn upload(
        &self,
        server: &str,
        directory: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Reply, String>;
}

pub struct RealPanel {
    state: shared::State,
    caller: Caller,
    index: Arc<tokio::sync::OnceCell<Arc<Index>>>,
}

impl RealPanel {
    pub fn new(
        state: shared::State,
        caller: Caller,
        index: Arc<tokio::sync::OnceCell<Arc<Index>>>,
    ) -> Self {
        Self {
            state,
            caller,
            index,
        }
    }

    fn request_builder(&self, method: Method, uri: &str) -> axum::http::request::Builder {
        let mut builder = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(header::ACCEPT, "application/json");

        for name in FORWARDED_HEADERS {
            if let Some(value) = self.caller.headers.get(name) {
                builder = builder.header(name, value.clone());
            }
        }

        builder
    }

    async fn send(
        &self,
        mut request: axum::http::Request<axum::body::Body>,
    ) -> Result<Reply, String> {
        request
            .extensions_mut()
            .insert(axum::extract::ConnectInfo(self.caller.addr));

        let response = self
            .state
            .send_authenticated_router_oneshot(
                request,
                self.caller.user.clone(),
                self.caller.auth.clone(),
            )
            .await
            .map_err(|err| format!("failed to run the panel route: {err}"))?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);

        let body = match axum::body::to_bytes(response.into_body(), crate::mcp::MAX_RESPONSE).await {
            Ok(body) => body,
            Err(err) => {
                let message = err.to_string();
                return Err(if message.contains("length limit") {
                    "response larger than 16 MiB".to_string()
                } else {
                    format!("failed to read the panel response: {message}")
                });
            }
        };

        Ok(Reply {
            status,
            content_type,
            body: body.to_vec(),
        })
    }

    async fn fetch_openapi(&self) -> Result<Reply, String> {
        // `/openapi.json` is not under `/api/client` or `/api/admin`, so it cannot go through
        // `call` and its user-supplied path validation.
        let request = self
            .request_builder(Method::GET, "/openapi.json")
            .body(axum::body::Body::empty())
            .map_err(|err| format!("failed to build the openapi request: {err}"))?;

        self.send(request).await
    }
}

#[async_trait::async_trait]
impl PanelApi for RealPanel {
    async fn call(
        &self,
        method: Method,
        path: &str,
        query: Option<&Map<String, Value>>,
        body: Option<&Value>,
    ) -> Result<Reply, String> {
        validate_path(path)?;

        let mut uri = path.to_string();
        if let Some(query) = query {
            let encoded = encode_query(query)?;
            if !encoded.is_empty() {
                uri.push('?');
                uri.push_str(&encoded);
            }
        }

        let (payload, content_type) = match body {
            None | Some(Value::Null) => (Vec::new(), None),
            Some(Value::String(text)) => (text.clone().into_bytes(), Some("text/plain; charset=utf-8")),
            Some(body) => (
                serde_json::to_vec(body)
                    .map_err(|err| format!("failed to serialize the request body: {err}"))?,
                Some("application/json"),
            ),
        };

        let mut builder = self.request_builder(method, &uri);
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }

        let request = builder
            .body(axum::body::Body::from(payload))
            .map_err(|err| format!("failed to build the panel request: {err}"))?;

        self.send(request).await
    }

    async fn index(&self) -> Result<Arc<Index>, String> {
        self.index
            .get_or_try_init(|| async {
                let reply = self.fetch_openapi().await?;
                if reply.status != 200 {
                    return Err(format!(
                        "failed to read the panel openapi document: HTTP {}",
                        reply.status
                    ));
                }

                let document: Value = serde_json::from_slice(&reply.body)
                    .map_err(|err| format!("failed to parse the panel openapi document: {err}"))?;

                Index::parse(&document).map(Arc::new)
            })
            .await
            .cloned()
    }

    async fn upload(
        &self,
        server: &str,
        directory: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Reply, String> {
        if file_name.is_empty() || file_name == "." || file_name == ".." {
            return Err("`file` must be a file name, not empty, `.` or `..`".to_string());
        }
        if file_name.contains('/') {
            return Err("`file` must be a file name without a directory separator".to_string());
        }

        let total_size = bytes.len();
        let reply = self
            .call(
                Method::GET,
                &format!("/api/client/servers/{server}/files/upload"),
                None,
                None,
            )
            .await?;

        if reply.status >= 400 {
            return Ok(reply);
        }

        let document: Value = serde_json::from_slice(&reply.body)
            .map_err(|err| format!("failed to parse the panel upload response: {err}"))?;
        let url = document
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| "the panel upload response did not contain a url".to_string())?;

        let mut url = url::Url::parse(url)
            .map_err(|err| format!("failed to parse the panel upload url: {err}"))?;
        url.query_pairs_mut()
            .append_pair("directory", directory)
            .append_pair("total_size", &total_size.to_string());

        let form = reqwest::multipart::Form::new().part(
            "files",
            reqwest::multipart::Part::bytes(bytes).file_name(file_name.to_string()),
        );

        let response = self
            .state
            .client
            .post(url)
            .multipart(form)
            .send()
            .await
            .map_err(|err| {
                format!(
                    "failed to reach the node upload endpoint (the panel must be able to reach the \
                     node's advertised url): {err}"
                )
            })?;

        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = response
            .bytes()
            .await
            .map_err(|err| format!("failed to read the node upload response: {err}"))?;
        let body = body[..body.len().min(MAX_UPLOAD_RESPONSE)].to_vec();

        Ok(Reply {
            status,
            content_type,
            body,
        })
    }
}

pub fn validate_path(path: &str) -> Result<(), String> {
    if !path.starts_with("/api/client/") && !path.starts_with("/api/admin/") {
        return Err("path must start with /api/client/ or /api/admin/".to_string());
    }

    if path.starts_with(&format!("/api/client/extensions/{}", crate::PACKAGE)) {
        return Err("path must not point at the mcp extension itself".to_string());
    }

    for segment in path.split('/').skip(1) {
        if segment.is_empty() {
            return Err("path must not contain empty segments".to_string());
        }

        if segment == "." || segment == ".." {
            return Err("path must not contain relative segments".to_string());
        }

        if segment.contains('{') || segment.contains('}') {
            return Err(format!(
                "replace the placeholder \"{segment}\" with a real value"
            ));
        }

        if segment.contains('?') || segment.contains('#') {
            return Err("send query parameters through the query object".to_string());
        }

        if !segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._~%!$&'()*+,;=:@-".contains(&b))
        {
            return Err(format!(
                "path segment \"{segment}\" contains a character that is not allowed"
            ));
        }
    }

    Ok(())
}

pub(crate) fn encode_query(query: &Map<String, Value>) -> Result<String, String> {
    fn push_value(
        serializer: &mut url::form_urlencoded::Serializer<'_, String>,
        key: &str,
        value: &Value,
    ) -> Result<(), String> {
        match value {
            Value::String(text) => {
                serializer.append_pair(key, text);
            }
            Value::Bool(flag) => {
                serializer.append_pair(key, if *flag { "true" } else { "false" });
            }
            Value::Number(_) => {
                serializer.append_pair(key, &value.to_string());
            }
            _ => {
                return Err(format!(
                    "query parameter \"{key}\" must be a string, number, boolean or a list of those"
                ));
            }
        }

        Ok(())
    }

    let mut serializer = url::form_urlencoded::Serializer::new(String::new());

    for (key, value) in query {
        match value {
            Value::Null => {}
            Value::Array(items) => {
                for item in items {
                    if !item.is_null() {
                        push_value(&mut serializer, key, item)?;
                    }
                }
            }
            value => push_value(&mut serializer, key, value)?,
        }
    }

    Ok(serializer.finish())
}
