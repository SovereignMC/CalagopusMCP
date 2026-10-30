use serde_json::{Map, Value, json};

use super::{
    SERVER_NAME,
    protocol::{self, Request, SUPPORTED_VERSIONS},
};
use crate::{panel::PanelApi, tools};

const INSTRUCTIONS: &str = "This server is the Calagopus Panel. Every tool call runs inside the panel \
with the permissions of the API key that connected you: nothing here can do more than that key \
allows, and the panel's own permission checks, account gates, rate limits and activity logging \
apply to each call.

Server work has dedicated tools. Use `list_servers` to find a server, `get_server` for its \
details and live resource usage, `power_server` for start/stop/restart/kill, `send_command` and \
`read_console` for the console, the `list_files`/`read_file`/`download_file`/`write_file`/\
`upload_file`/`delete_files`/`rename_file`/`create_directory`/`pull_file` tools for file \
management, and `list_backups`/`create_backup`/`restore_backup`/`delete_backup` for backups.

Every other panel endpoint, client and admin, is reachable through the meta tools. The workflow \
is: `search_api` to find candidate operations, `describe_api` to read one operation's \
parameters, request body and responses, then `read_api` or `write_api` to execute it. A path \
returned by `search_api` contains `{placeholders}`: replace every one of them with a real value \
before calling `read_api` or `write_api`, and send query parameters through the `query` object \
rather than inside the path. `/api/admin` operations need admin permissions on the API key.";

pub async fn dispatch(panel: &dyn PanelApi, request: Request) -> Value {
    match request.method.as_str() {
        "initialize" => {
            let result = initialize(&request.params);
            success(request.id, result)
        }
        "ping" => success(request.id, json!({})),
        "tools/list" => success(request.id, json!({ "tools": tools::definitions() })),
        "tools/call" => tools_call(panel, request).await,
        method => protocol::error_response(
            request.id,
            -32601,
            format!("method not found: {method}"),
        ),
    }
}

fn success(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn initialize(params: &Map<String, Value>) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let version = if SUPPORTED_VERSIONS.contains(&requested) {
        requested
    } else {
        protocol::LATEST_VERSION
    };

    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": { "listChanged": false },
        },
        "serverInfo": {
            "name": SERVER_NAME,
            "title": "Calagopus MCP",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": INSTRUCTIONS,
    })
}

async fn tools_call(panel: &dyn PanelApi, request: Request) -> Value {
    let Some(Value::String(name)) = request.params.get("name").cloned() else {
        return protocol::error_response(
            request.id,
            -32602,
            "invalid params: `name` must be a string",
        );
    };

    let arguments = match request.params.get("arguments") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(arguments)) => arguments.clone(),
        Some(_) => {
            return protocol::error_response(
                request.id,
                -32602,
                "invalid params: `arguments` must be an object",
            );
        }
    };

    match tools::call(panel, &name, &arguments).await {
        tools::Outcome::UnknownTool => {
            protocol::error_response(request.id, -32602, format!("unknown tool: {name}"))
        }
        tools::Outcome::Result(result) => success(request.id, result),
    }
}
