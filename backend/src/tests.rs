use std::sync::{Arc, Mutex};

use axum::{
    body::{Bytes, to_bytes},
    http::{Method, header},
};
use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::{
    mcp::{MAX_TEXT, dispatch, protocol},
    openapi::Index,
    panel::{PanelApi, Reply, encode_query, validate_path},
    tools::{self, Outcome, output},
};

struct RecordedCall {
    method: Method,
    path: String,
    query: Option<Map<String, Value>>,
    body: Option<Value>,
}

struct RecordedUpload {
    server: String,
    directory: String,
    file: String,
    bytes: Vec<u8>,
}

struct MockPanel {
    calls: Mutex<Vec<RecordedCall>>,
    uploads: Mutex<Vec<RecordedUpload>>,
    routes: Mutex<Vec<(Method, String, Reply)>>,
    index: Arc<Index>,
}

impl MockPanel {
    fn new(index: Arc<Index>) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            uploads: Mutex::new(Vec::new()),
            routes: Mutex::new(Vec::new()),
            index,
        }
    }

    fn route(self, method: Method, path: &str, reply: Reply) -> Self {
        self.routes
            .lock()
            .unwrap()
            .push((method, path.to_string(), reply));
        self
    }

    fn last_call(&self) -> (Method, String, Option<Map<String, Value>>, Option<Value>) {
        let calls = self.calls.lock().unwrap();
        let call = calls.last().expect("no panel call was recorded");

        (
            call.method.clone(),
            call.path.clone(),
            call.query.clone(),
            call.body.clone(),
        )
    }
}

#[async_trait::async_trait]
impl PanelApi for MockPanel {
    async fn call(
        &self,
        method: Method,
        path: &str,
        query: Option<&Map<String, Value>>,
        body: Option<&Value>,
    ) -> Result<Reply, String> {
        self.calls.lock().unwrap().push(RecordedCall {
            method: method.clone(),
            path: path.to_string(),
            query: query.cloned(),
            body: body.cloned(),
        });

        let routes = self.routes.lock().unwrap();
        for (route_method, route_path, reply) in routes.iter() {
            if *route_method == method && route_path == path {
                return Ok(Reply {
                    status: reply.status,
                    content_type: reply.content_type.clone(),
                    body: reply.body.clone(),
                });
            }
        }

        Ok(json_reply(&json!({})))
    }

    async fn index(&self) -> Result<Arc<Index>, String> {
        Ok(self.index.clone())
    }

    async fn upload(
        &self,
        server: &str,
        directory: &str,
        file_name: &str,
        bytes: Vec<u8>,
    ) -> Result<Reply, String> {
        self.uploads.lock().unwrap().push(RecordedUpload {
            server: server.to_string(),
            directory: directory.to_string(),
            file: file_name.to_string(),
            bytes,
        });

        Ok(json_reply(&json!({})))
    }
}

fn json_reply(value: &Value) -> Reply {
    Reply {
        status: 200,
        content_type: Some("application/json".to_string()),
        body: serde_json::to_vec(value).unwrap(),
    }
}

fn text_reply(status: u16, body: &str) -> Reply {
    Reply {
        status,
        content_type: Some("text/plain; charset=utf-8".to_string()),
        body: body.as_bytes().to_vec(),
    }
}

fn arguments(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        value => panic!("arguments must be an object, got {value}"),
    }
}

const SPEC: &str = r##"{
    "paths": {
        "/api/client/servers": {
            "get": {
                "summary": "List servers",
                "description": "Lists every server the caller can see.",
                "operationId": "get_api_client_servers",
                "tags": ["/api/client"],
                "parameters": [
                    { "name": "page", "in": "query", "required": false, "schema": { "type": "integer" } },
                    { "name": "search", "in": "query", "required": false, "schema": { "type": "string" } }
                ],
                "responses": { "200": { "description": "" } }
            }
        },
        "/api/admin/users/{user}": {
            "delete": {
                "summary": "Delete user",
                "description": "Deletes a panel user.",
                "operationId": "delete_api_admin_users_user",
                "tags": ["/api/admin"],
                "parameters": [
                    { "name": "user", "in": "path", "required": true, "schema": { "type": "string" } }
                ],
                "requestBody": {
                    "content": {
                        "application/json": {
                            "schema": { "$ref": "#/components/schemas/UserDelete" }
                        }
                    }
                },
                "responses": {
                    "200": {
                        "description": "",
                        "content": {
                            "application/json": {
                                "schema": { "$ref": "#/components/schemas/ApiError" }
                            }
                        }
                    }
                }
            }
        }
    },
    "components": {
        "schemas": {
            "ApiError": { "type": "object" },
            "UserDelete": {
                "type": "object",
                "properties": { "error": { "$ref": "#/components/schemas/ApiError" } }
            },
            "Unrelated": { "type": "object" }
        }
    }
}"##;

fn index() -> Arc<Index> {
    Arc::new(Index::parse(&serde_json::from_str(SPEC).unwrap()).unwrap())
}

fn panel() -> MockPanel {
    MockPanel::new(index())
}

fn request(body: &str) -> protocol::Request {
    match protocol::prepare(body.as_bytes()) {
        protocol::Prepared::Request(request) => request,
        _ => panic!("expected a request"),
    }
}

#[test]
fn prepare_accepts_a_request() {
    let request = request(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);

    assert_eq!(request.id, json!(7));
    assert_eq!(request.method, "ping");
    assert!(request.params.is_empty());
}

#[test]
fn prepare_accepts_a_string_id() {
    let request = request(r#"{"jsonrpc":"2.0","id":"a1","method":"tools/list","params":{}}"#);

    assert_eq!(request.id, json!("a1"));
}

#[test]
fn prepare_treats_a_message_without_id_as_a_notification() {
    assert!(matches!(
        protocol::prepare(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        protocol::Prepared::Notification
    ));
}

#[test]
fn prepare_rejects_malformed_json() {
    match protocol::prepare(b"{not json") {
        protocol::Prepared::Invalid { status, error } => {
            assert_eq!(status, 400);
            assert_eq!(error["error"]["code"], json!(-32700));
        }
        _ => panic!("expected an invalid response"),
    }
}

#[test]
fn prepare_rejects_a_batch() {
    match protocol::prepare(br#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#) {
        protocol::Prepared::Invalid { status, error } => {
            assert_eq!(status, 400);
            assert_eq!(error["error"]["code"], json!(-32600));
        }
        _ => panic!("expected an invalid response"),
    }
}

#[test]
fn prepare_rejects_a_foreign_jsonrpc_version() {
    match protocol::prepare(br#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#) {
        protocol::Prepared::Invalid { status, error } => {
            assert_eq!(status, 200);
            assert_eq!(error["error"]["code"], json!(-32600));
        }
        _ => panic!("expected an invalid response"),
    }
}

#[test]
fn prepare_rejects_non_object_params() {
    match protocol::prepare(br#"{"jsonrpc":"2.0","id":1,"method":"ping","params":"x"}"#) {
        protocol::Prepared::Invalid { status, error } => {
            assert_eq!(status, 200);
            assert_eq!(error["error"]["code"], json!(-32602));
        }
        _ => panic!("expected an invalid response"),
    }
}

#[tokio::test]
async fn initialize_negotiates_the_requested_version() {
    let panel = panel();
    let response = dispatch::dispatch(
        &panel,
        self::request(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
        ),
    )
    .await;

    assert_eq!(response["result"]["protocolVersion"], json!("2025-06-18"));
    assert_eq!(response["result"]["serverInfo"]["name"], json!("calagopus-mcp"));
    assert_eq!(response["id"], json!(1));
}

#[tokio::test]
async fn initialize_falls_back_to_the_latest_version() {
    let panel = panel();
    let response = dispatch::dispatch(
        &panel,
        self::request(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#,
        ),
    )
    .await;

    assert_eq!(response["result"]["protocolVersion"], json!("2025-11-25"));
}

#[tokio::test]
async fn unknown_method_is_a_jsonrpc_error() {
    let panel = panel();
    let response =
        dispatch::dispatch(&panel, self::request(r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#))
            .await;

    assert_eq!(response["error"]["code"], json!(-32601));
}

#[tokio::test]
async fn unknown_tool_is_a_jsonrpc_error() {
    let panel = panel();
    let response = dispatch::dispatch(
        &panel,
        self::request(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"nope"}}"#,
        ),
    )
    .await;

    assert_eq!(response["error"]["code"], json!(-32602));
}

#[test]
fn validate_path_accepts_client_and_admin_paths() {
    assert!(validate_path("/api/client/servers").is_ok());
    assert!(validate_path("/api/client/servers/abc/files/list").is_ok());
    assert!(validate_path("/api/admin/users").is_ok());
}

#[test]
fn validate_path_rejects_foreign_prefixes() {
    assert_eq!(
        validate_path("/api/remote/servers").unwrap_err(),
        "path must start with /api/client/ or /api/admin/"
    );
}

#[test]
fn validate_path_rejects_relative_segments() {
    assert!(validate_path("/api/client/servers/../admin/x").is_err());
    assert!(validate_path("/api/client/./servers").is_err());
}

#[test]
fn validate_path_names_the_placeholder() {
    assert_eq!(
        validate_path("/api/client/servers/{server}/files/list").unwrap_err(),
        "replace the placeholder \"{server}\" with a real value"
    );
}

#[test]
fn validate_path_rejects_inline_queries() {
    assert_eq!(
        validate_path("/api/client/servers?page=1").unwrap_err(),
        "send query parameters through the query object"
    );
}

#[test]
fn validate_path_rejects_self_calls() {
    assert!(validate_path("/api/client/extensions/dev_sovereignmc_mcpserver/mcp").is_err());
}

#[test]
fn validate_path_rejects_empty_segments() {
    assert!(validate_path("/api/client/servers//files").is_err());
}

#[test]
fn encode_query_appends_lists_and_skips_nulls() {
    let query = arguments(json!({
        "files": ["a", "b"],
        "n": 1,
        "skip": null,
    }));

    assert_eq!(encode_query(&query).unwrap(), "files=a&files=b&n=1");
}

#[test]
fn encode_query_rejects_nested_values() {
    let query = arguments(json!({ "nested": { "a": 1 } }));

    assert_eq!(
        encode_query(&query).unwrap_err(),
        "query parameter \"nested\" must be a string, number, boolean or a list of those"
    );
}

#[test]
fn render_returns_text_bodies_verbatim() {
    assert_eq!(output::render(&text_reply(200, "hello")), "hello");
}

#[test]
fn render_prefixes_the_status_for_errors() {
    let reply = Reply {
        status: 403,
        content_type: Some("application/json".to_string()),
        body: br#"{"error":"nope"}"#.to_vec(),
    };

    assert_eq!(output::render(&reply), r#"HTTP 403: {"error":"nope"}"#);
}

#[test]
fn render_describes_binary_bodies() {
    let reply = Reply {
        status: 200,
        content_type: Some("application/octet-stream".to_string()),
        body: vec![0xff, 0x00],
    };

    assert_eq!(
        output::render(&reply),
        "Binary response (application/octet-stream, 2 bytes)"
    );
}

#[test]
fn render_truncates_at_the_text_cap() {
    let reply = text_reply(200, &"a".repeat(200_000));
    let rendered = output::render(&reply);

    assert!(rendered.starts_with(&"a".repeat(MAX_TEXT)));
    assert!(rendered.contains("[Truncated: showing the first 100000 of 200000 bytes…]"));
}

#[test]
fn render_truncates_on_a_character_boundary() {
    let reply = text_reply(200, &format!("a{}", "é".repeat(60_000)));
    let rendered = output::render(&reply);

    assert!(rendered.contains("[Truncated: showing the first 99999 of 120001 bytes…]"));
}

#[test]
fn index_parses_every_operation() {
    let index = index();

    assert_eq!(index.operations.len(), 2);
    assert_eq!(index.schemas["ApiError"]["type"], json!("object"));
}

#[test]
fn index_search_matches_paths_and_lists_query_parameters() {
    let index = index();
    let matches = index.search("servers", "all", 15);

    assert_eq!(
        matches,
        vec!["GET /api/client/servers — List servers (query: page, search)"]
    );
}

#[test]
fn index_search_marks_operations_with_a_body() {
    let index = index();
    let matches = index.search("delete user", "admin", 15);

    assert_eq!(
        matches,
        vec!["DELETE /api/admin/users/{user} — Delete user (body)"]
    );
}

#[test]
fn index_search_respects_scope() {
    let index = index();

    assert!(index.search("servers", "admin", 15).is_empty());
    assert!(index.search("delete", "client", 15).is_empty());
}

#[test]
fn index_find_is_exact_on_the_template_path() {
    let index = index();

    assert!(index.find("GET", "/api/client/servers").is_some());
    assert!(index.find("get", "/api/client/servers").is_some());
    assert!(index.find("GET", "/api/client/servers/abc").is_none());
}

#[test]
fn index_describe_includes_the_referenced_schemas_only() {
    let index = index();
    let operation = index
        .find("DELETE", "/api/admin/users/{user}")
        .expect("operation missing");
    let described = index.describe(operation);

    assert!(described.contains("\"operation_id\": \"delete_api_admin_users_user\""));
    assert!(described.contains("\"request_body\""));
    assert!(described.contains("\"UserDelete\""));
    assert!(described.contains("\"ApiError\""));
    assert!(!described.contains("\"Unrelated\""));
}

#[tokio::test]
async fn list_servers_issues_a_paged_panel_request() {
    let panel = panel();
    let outcome = tools::call(
        &panel,
        "list_servers",
        &arguments(json!({ "page": 2, "per_page": 50, "other": true })),
    )
    .await;

    assert!(matches!(outcome, Outcome::Result(_)));

    let (method, path, query, body) = panel.last_call();
    assert_eq!(method, Method::GET);
    assert_eq!(path, "/api/client/servers");
    assert_eq!(
        query,
        Some(arguments(json!({ "page": 2, "per_page": 50, "other": true })))
    );
    assert!(body.is_none());
}

#[tokio::test]
async fn power_server_posts_the_action() {
    let panel = panel();
    let outcome = tools::call(
        &panel,
        "power_server",
        &arguments(json!({ "server": "abc", "action": "start" })),
    )
    .await;

    assert!(matches!(outcome, Outcome::Result(_)));

    let (method, path, query, body) = panel.last_call();
    assert_eq!(method, Method::POST);
    assert_eq!(path, "/api/client/servers/abc/power");
    assert!(query.is_none());
    assert_eq!(body, Some(json!({ "action": "start" })));
}

#[tokio::test]
async fn power_server_rejects_an_unknown_action() {
    let panel = panel();
    let Outcome::Result(result) = tools::call(
        &panel,
        "power_server",
        &arguments(json!({ "server": "abc", "action": "explode" })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    assert_eq!(result["isError"], json!(true));
    assert_eq!(
        result["content"][0]["text"],
        json!("`action` must be start, stop, restart or kill")
    );
}

#[tokio::test]
async fn upload_file_decodes_base64_and_records_the_upload() {
    let panel = panel();
    let encoded = base64::engine::general_purpose::STANDARD.encode(b"hello world");
    let outcome = tools::call(
        &panel,
        "upload_file",
        &arguments(json!({
            "server": "abc",
            "directory": "/plugins",
            "file": "hello.txt",
            "content_base64": encoded,
        })),
    )
    .await;

    assert!(matches!(outcome, Outcome::Result(_)));

    let uploads = panel.uploads.lock().unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].server, "abc");
    assert_eq!(uploads[0].directory, "/plugins");
    assert_eq!(uploads[0].file, "hello.txt");
    assert_eq!(uploads[0].bytes, b"hello world");
}

#[tokio::test]
async fn unknown_tool_is_reported_to_the_transport() {
    let panel = panel();

    assert!(matches!(
        tools::call(&panel, "nope", &Map::new()).await,
        Outcome::UnknownTool
    ));
}

#[tokio::test]
async fn missing_argument_is_a_tool_error() {
    let panel = panel();
    let Outcome::Result(result) = tools::call(
        &panel,
        "send_command",
        &arguments(json!({ "server": "abc" })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["content"][0]["text"], json!("`command` is required"));
    assert!(panel.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn read_console_strips_ansi_and_tails() {
    let panel = MockPanel::new(index()).route(
        Method::GET,
        "/api/client/servers/abc/logs",
        text_reply(200, "\u{1b}[32mone\u{1b}[0m\r\ntwo\nthree\n"),
    );

    let Outcome::Result(result) = tools::call(
        &panel,
        "read_console",
        &arguments(json!({ "server": "abc", "lines": 2 })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    assert_eq!(result["content"][0]["text"], json!("two\nthree"));
}

#[tokio::test]
async fn read_file_reports_a_binary_file() {
    let panel = MockPanel::new(index()).route(
        Method::GET,
        "/api/client/servers/abc/files/contents",
        Reply {
            status: 200,
            content_type: Some("application/octet-stream".to_string()),
            body: vec![0xff, 0xfe],
        },
    );

    let Outcome::Result(result) = tools::call(
        &panel,
        "read_file",
        &arguments(json!({ "server": "abc", "file": "/world.dat" })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    assert_eq!(
        result["content"][0]["text"],
        json!("Binary file (2 bytes); use download_file")
    );
}

#[tokio::test]
async fn get_server_merges_details_and_resources() {
    let panel = MockPanel::new(index())
        .route(
            Method::GET,
            "/api/client/servers/abc",
            json_reply(&json!({ "server": { "uuid": "abc", "name": "survival" } })),
        )
        .route(
            Method::GET,
            "/api/client/servers/abc/resources",
            json_reply(&json!({ "resources": { "state": "running" } })),
        );

    let Outcome::Result(result) = tools::call(
        &panel,
        "get_server",
        &arguments(json!({ "server": "abc" })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    let text = result["content"][0]["text"].as_str().unwrap();
    let value: Value = serde_json::from_str(text).unwrap();

    assert_eq!(value["server"]["uuid"], json!("abc"));
    assert_eq!(value["server"]["name"], json!("survival"));
    assert_eq!(value["resources"]["state"], json!("running"));
}

#[tokio::test]
async fn get_server_keeps_details_when_resources_fail() {
    let panel = MockPanel::new(index())
        .route(
            Method::GET,
            "/api/client/servers/abc",
            json_reply(&json!({ "uuid": "abc" })),
        )
        .route(
            Method::GET,
            "/api/client/servers/abc/resources",
            text_reply(404, "no resource usage data found for server"),
        );

    let Outcome::Result(result) = tools::call(
        &panel,
        "get_server",
        &arguments(json!({ "server": "abc" })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    let text = result["content"][0]["text"].as_str().unwrap();
    let value: Value = serde_json::from_str(text).unwrap();

    assert_eq!(value["server"]["uuid"], json!("abc"));
    assert_eq!(
        value["resources"]["error"],
        json!("HTTP 404: no resource usage data found for server")
    );
}

#[tokio::test]
async fn panel_errors_become_tool_errors() {
    let panel = MockPanel::new(index()).route(
        Method::GET,
        "/api/client/servers/abc/files/list",
        text_reply(403, "missing permission files.read"),
    );

    let Outcome::Result(result) = tools::call(
        &panel,
        "list_files",
        &arguments(json!({ "server": "abc" })),
    )
    .await
    else {
        panic!("expected a tool result");
    };

    assert_eq!(result["isError"], json!(true));
    assert_eq!(
        result["content"][0]["text"],
        json!("HTTP 403: missing permission files.read")
    );
}

async fn body_of(response: axum::response::Response) -> Value {
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body too large");
    serde_json::from_slice(&body).expect("response body is not json")
}

#[tokio::test]
async fn transport_answers_a_request_over_http() {
    let panel = panel();
    let response = crate::mcp::respond(
        Method::POST,
        Bytes::from_static(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        ),
        &panel,
    )
    .await;

    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE].to_str().unwrap(),
        "application/json"
    );
    assert_eq!(
        response.headers()[header::CACHE_CONTROL].to_str().unwrap(),
        "no-store"
    );

    let value = body_of(response).await;
    assert_eq!(value["result"]["protocolVersion"], json!("2025-03-26"));
    assert_eq!(value["jsonrpc"], json!("2.0"));
}

#[tokio::test]
async fn transport_refuses_a_non_post() {
    let panel = panel();
    let response = crate::mcp::respond(Method::GET, Bytes::new(), &panel).await;

    assert_eq!(response.status(), 405);
    assert_eq!(response.headers()[header::ALLOW].to_str().unwrap(), "POST");
}

#[tokio::test]
async fn transport_acknowledges_a_notification() {
    let panel = panel();
    let response = crate::mcp::respond(
        Method::POST,
        Bytes::from_static(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        &panel,
    )
    .await;

    assert_eq!(response.status(), 202);
    assert!(to_bytes(response.into_body(), 1024).await.unwrap().is_empty());
}

#[tokio::test]
async fn transport_rejects_malformed_json_with_http_400() {
    let panel = panel();
    let response = crate::mcp::respond(Method::POST, Bytes::from_static(b"{"), &panel).await;

    assert_eq!(response.status(), 400);
    assert_eq!(body_of(response).await["error"]["code"], json!(-32700));
}

#[test]
fn definitions_expose_every_documented_tool() {
    const EXPECTED: [&str; 22] = [
        "search_api",
        "describe_api",
        "read_api",
        "write_api",
        "list_servers",
        "get_server",
        "power_server",
        "send_command",
        "read_console",
        "list_files",
        "read_file",
        "download_file",
        "write_file",
        "upload_file",
        "delete_files",
        "rename_file",
        "create_directory",
        "pull_file",
        "list_backups",
        "create_backup",
        "restore_backup",
        "delete_backup",
    ];

    let definitions = tools::definitions();
    let names: Vec<&str> = definitions
        .iter()
        .map(|definition| definition["name"].as_str().expect("tool without a name"))
        .collect();

    assert_eq!(names, EXPECTED);

    for definition in &definitions {
        let input_schema = &definition["inputSchema"];
        let properties = input_schema["properties"]
            .as_object()
            .expect("tool without an input schema");

        assert_eq!(input_schema["type"], json!("object"), "{names:?}");
        assert_eq!(input_schema["additionalProperties"], json!(false));
        assert_eq!(definition["annotations"]["openWorldHint"], json!(false));
        assert!(!definition["title"].as_str().unwrap_or_default().is_empty());
        assert!(!definition["description"].as_str().unwrap_or_default().is_empty());

        for required in input_schema["required"].as_array().expect("required list") {
            let required = required.as_str().expect("required entry is not a string");
            assert!(
                properties.contains_key(required),
                "{} requires undeclared property {required}",
                definition["name"]
            );
        }
    }
}

#[tokio::test]
async fn every_documented_tool_is_dispatchable() {
    let panel = panel();

    for definition in tools::definitions() {
        let name = definition["name"].as_str().expect("tool without a name");

        assert!(
            !matches!(
                tools::call(&panel, name, &Map::new()).await,
                Outcome::UnknownTool
            ),
            "{name} is in the catalogue but not dispatched"
        );
    }
}
