pub mod args;
pub mod definitions;
pub mod output;

use std::borrow::Cow;

use axum::http::Method;
use base64::Engine as _;
use serde_json::{Map, Value, json};

use crate::{
    mcp::{MAX_DOWNLOAD, MAX_TEXT, MAX_UPLOAD},
    panel::{PanelApi, Reply},
};

pub use definitions::definitions;

pub enum Outcome {
    Result(Value),
    UnknownTool,
}

type ToolResult = Result<String, String>;

const DELETE: &str = "DELETE";
const GET: &str = "GET";
const POST: &str = "POST";
const PUT: &str = "PUT";

pub async fn call(panel: &dyn PanelApi, name: &str, arguments: &Map<String, Value>) -> Outcome {
    let result = match name {
        "search_api" => search_api(panel, arguments).await,
        "describe_api" => describe_api(panel, arguments).await,
        "read_api" => read_api(panel, arguments).await,
        "write_api" => write_api(panel, arguments).await,
        "list_servers" => list_servers(panel, arguments).await,
        "get_server" => get_server(panel, arguments).await,
        "power_server" => power_server(panel, arguments).await,
        "send_command" => send_command(panel, arguments).await,
        "read_console" => read_console(panel, arguments).await,
        "list_files" => list_files(panel, arguments).await,
        "read_file" => read_file(panel, arguments).await,
        "download_file" => download_file(panel, arguments).await,
        "write_file" => write_file(panel, arguments).await,
        "upload_file" => upload_file(panel, arguments).await,
        "delete_files" => delete_files(panel, arguments).await,
        "rename_file" => rename_file(panel, arguments).await,
        "create_directory" => create_directory(panel, arguments).await,
        "pull_file" => pull_file(panel, arguments).await,
        "list_backups" => list_backups(panel, arguments).await,
        "create_backup" => create_backup(panel, arguments).await,
        "restore_backup" => restore_backup(panel, arguments).await,
        "delete_backup" => delete_backup(panel, arguments).await,
        _ => return Outcome::UnknownTool,
    };

    Outcome::Result(match result {
        Ok(text) => json!({
            "content": [{ "type": "text", "text": text }],
        }),
        Err(message) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
    })
}

async fn search_api(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let query = args::text(arguments, "query")?;
    let scope = args::choice(arguments, "scope", Some("all"), &["all", "client", "admin"])?;
    let limit = args::integer(arguments, "limit", 15, 1, 50)?;

    let index = panel.index().await?;
    let matches = index.search(&query, &scope, limit as usize);

    if matches.is_empty() {
        return Ok(format!(
            "No panel API operation matched \"{query}\" in scope \"{scope}\". Try fewer or different \
             words, or scope \"all\"."
        ));
    }

    Ok(matches.join("\n"))
}

async fn describe_api(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let method = args::choice(
        arguments,
        "method",
        None,
        &["GET", "POST", "PUT", "PATCH", "DELETE"],
    )?;
    let path = args::text(arguments, "path")?;

    let index = panel.index().await?;
    let operation = index.find(&method, &path).ok_or_else(|| {
        format!(
            "no {method} operation on {path}; use search_api to find the right path and method"
        )
    })?;

    Ok(output::truncate(&index.describe(operation)))
}

async fn read_api(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let path = args::text(arguments, "path")?;
    let query = args::object(arguments, "query")?;

    request(panel, GET, &path, query.as_ref(), None).await
}

async fn write_api(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let method = args::choice(arguments, "method", None, &["POST", "PUT", "PATCH", "DELETE"])?;
    let path = args::text(arguments, "path")?;
    let query = args::object(arguments, "query")?;
    let body = arguments.get("body").filter(|body| !body.is_null());

    request(panel, &method, &path, query.as_ref(), body).await
}

async fn list_servers(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let page = args::integer(arguments, "page", 1, 1, args::MAX_PAGE)?;
    let per_page = args::integer(arguments, "per_page", 25, 1, 100)?;
    let other = args::boolean(arguments, "other", false)?;
    let search = args::optional_text(arguments, "search")?;

    let mut query = Map::new();
    query.insert("page".to_string(), json!(page));
    query.insert("per_page".to_string(), json!(per_page));
    query.insert("other".to_string(), json!(other));
    if let Some(search) = search {
        query.insert("search".to_string(), json!(search));
    }

    request(
        panel,
        GET,
        "/api/client/servers",
        Some(&query),
        None,
    )
    .await
}

async fn get_server(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;

    let details = panel
        .call(Method::GET, &server_path(&server, ""), None, None)
        .await?;
    if details.status >= 400 {
        return Err(output::render(&details));
    }
    let mut details = parse_json(&details, "server details")?;
    if let Some(server) = details.get("server").cloned() {
        details = server;
    }

    let resources = panel
        .call(Method::GET, &server_path(&server, "/resources"), None, None)
        .await?;
    let resources = if resources.status >= 400 {
        json!({ "error": output::render(&resources) })
    } else {
        match parse_json(&resources, "server resources")?.get("resources") {
            Some(resources) => resources.clone(),
            None => json!({ "error": "the panel returned no resources object" }),
        }
    };

    Ok(pretty(&json!({
        "server": details,
        "resources": resources,
    })))
}

async fn power_server(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let action = args::choice(
        arguments,
        "action",
        None,
        &["start", "stop", "restart", "kill"],
    )?;

    request(
        panel,
        POST,
        &server_path(&server, "/power"),
        None,
        Some(&json!({ "action": action })),
    )
    .await
}

async fn send_command(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let command = args::text(arguments, "command")?;

    let length = command.chars().count();
    if length == 0 || length > 1024 {
        return Err(format!(
            "`command` must be between 1 and 1024 characters, got {length}"
        ));
    }

    request(
        panel,
        POST,
        &server_path(&server, "/command"),
        None,
        Some(&json!({ "command": command })),
    )
    .await
}

async fn read_console(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let lines = args::integer(arguments, "lines", 100, 1, 1000)?;

    let mut query = Map::new();
    query.insert("lines".to_string(), json!(lines));

    let reply = panel
        .call(
            Method::GET,
            &server_path(&server, "/logs"),
            Some(&query),
            None,
        )
        .await?;
    if reply.status >= 400 {
        return Err(output::render(&reply));
    }

    let text = output::strip_ansi(&String::from_utf8_lossy(&reply.body));
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines as usize);

    Ok(output::truncate(&all[start..].join("\n")))
}

async fn list_files(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let directory = args::optional_text(arguments, "directory")?.unwrap_or_else(|| "/".to_string());
    let page = args::integer(arguments, "page", 1, 1, args::MAX_PAGE)?;
    let per_page = args::integer(arguments, "per_page", 100, 1, 100)?;
    let sort = args::optional_text(arguments, "sort")?;

    let mut query = Map::new();
    query.insert("directory".to_string(), json!(directory));
    query.insert("page".to_string(), json!(page));
    query.insert("per_page".to_string(), json!(per_page));
    if let Some(sort) = sort {
        query.insert("sort".to_string(), json!(sort));
    }

    request(
        panel,
        GET,
        &server_path(&server, "/files/list"),
        Some(&query),
        None,
    )
    .await
}

async fn read_file(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let file = args::text(arguments, "file")?;

    let reply = contents(panel, &server, &file).await?;
    if reply.status >= 400 {
        return Err(output::render(&reply));
    }

    match std::str::from_utf8(&reply.body) {
        Ok(text) => Ok(output::truncate(text)),
        Err(_) => Ok(format!(
            "Binary file ({} bytes); use download_file",
            reply.body.len()
        )),
    }
}

async fn download_file(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let file = args::text(arguments, "file")?;
    let max_bytes = args::integer(
        arguments,
        "max_bytes",
        MAX_DOWNLOAD as i64,
        1,
        MAX_UPLOAD as i64,
    )? as usize;

    let reply = contents(panel, &server, &file).await?;
    if reply.status >= 400 {
        return Err(output::render(&reply));
    }

    if reply.body.len() <= MAX_TEXT
        && let Ok(text) = std::str::from_utf8(&reply.body)
    {
        return Ok(text.to_string());
    }

    if reply.body.len() > max_bytes {
        return Err(format!(
            "file is {} bytes, larger than max_bytes ({max_bytes}); raise max_bytes or use \
             pull_file to move it without loading it",
            reply.body.len()
        ));
    }

    Ok(base64::engine::general_purpose::STANDARD.encode(&reply.body))
}

async fn write_file(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let file = args::text(arguments, "file")?;
    let content = args::text(arguments, "content")?;

    let mut query = Map::new();
    query.insert("file".to_string(), json!(file));

    request(
        panel,
        POST,
        &server_path(&server, "/files/write"),
        Some(&query),
        Some(&Value::String(content)),
    )
    .await
}

async fn upload_file(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let directory = args::optional_text(arguments, "directory")?.unwrap_or_else(|| "/".to_string());
    let file = args::text(arguments, "file")?;
    let encoded = args::text(arguments, "content_base64")?;

    let bytes = decode_base64(&encoded)?;
    if bytes.len() > MAX_UPLOAD {
        return Err(format!(
            "file is {} bytes, larger than the {MAX_UPLOAD} byte upload limit; use pull_file to \
             have the node download it from a url instead",
            bytes.len()
        ));
    }
    let total = bytes.len();

    let reply = panel.upload(&server, &directory, &file, bytes).await?;
    let rendered = output::render(&reply);
    if reply.status >= 400 {
        return Err(rendered);
    }

    Ok(format!("uploaded {file} to {directory} ({total} bytes)\n{rendered}"))
}

async fn delete_files(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let root = args::optional_text(arguments, "root")?.unwrap_or_else(|| "/".to_string());
    let files = args::string_list(arguments, "files")?;
    let permanent = args::boolean(arguments, "permanent", false)?;

    if files.is_empty() {
        return Err("`files` must contain at least one file or directory name".to_string());
    }

    request(
        panel,
        POST,
        &server_path(&server, "/files/delete"),
        None,
        Some(&json!({
            "root": root,
            "files": files,
            "permanent": permanent,
        })),
    )
    .await
}

async fn rename_file(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let root = args::optional_text(arguments, "root")?.unwrap_or_else(|| "/".to_string());
    let from = args::text(arguments, "from")?;
    let to = args::text(arguments, "to")?;

    request(
        panel,
        PUT,
        &server_path(&server, "/files/rename"),
        None,
        Some(&json!({
            "root": root,
            "files": [{ "from": from, "to": to }],
        })),
    )
    .await
}

async fn create_directory(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let root = args::optional_text(arguments, "root")?.unwrap_or_else(|| "/".to_string());
    let name = args::text(arguments, "name")?;

    request(
        panel,
        POST,
        &server_path(&server, "/files/create-directory"),
        None,
        Some(&json!({ "root": root, "name": name })),
    )
    .await
}

async fn pull_file(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let root = args::optional_text(arguments, "root")?.unwrap_or_else(|| "/".to_string());
    let url = args::text(arguments, "url")?;
    let name = args::optional_text(arguments, "name")?;
    let use_header = args::boolean(arguments, "use_header", false)?;
    let foreground = args::boolean(arguments, "foreground", false)?;

    let mut body = Map::new();
    body.insert("root".to_string(), json!(root));
    body.insert("url".to_string(), json!(url));
    body.insert("use_header".to_string(), json!(use_header));
    body.insert("foreground".to_string(), json!(foreground));
    if let Some(name) = name {
        body.insert("name".to_string(), json!(name));
    }

    request(
        panel,
        POST,
        &server_path(&server, "/files/pull"),
        None,
        Some(&Value::Object(body)),
    )
    .await
}

async fn list_backups(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let page = args::integer(arguments, "page", 1, 1, args::MAX_PAGE)?;
    let per_page = args::integer(arguments, "per_page", 10, 1, 100)?;
    let ungrouped = args::boolean(arguments, "ungrouped", false)?;
    let search = args::optional_text(arguments, "search")?;

    let mut query = Map::new();
    query.insert("page".to_string(), json!(page));
    query.insert("per_page".to_string(), json!(per_page));
    query.insert("ungrouped".to_string(), json!(ungrouped));
    if let Some(search) = search {
        query.insert("search".to_string(), json!(search));
    }

    request(
        panel,
        GET,
        &server_path(&server, "/backups"),
        Some(&query),
        None,
    )
    .await
}

async fn create_backup(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let name = args::optional_text(arguments, "name")?;
    let ignored_files = args::string_list(arguments, "ignored_files")?;

    let mut body = Map::new();
    body.insert("ignored_files".to_string(), json!(ignored_files));
    if let Some(name) = name {
        body.insert("name".to_string(), json!(name));
    }

    request(
        panel,
        POST,
        &server_path(&server, "/backups"),
        None,
        Some(&Value::Object(body)),
    )
    .await
}

async fn restore_backup(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let backup = args::server(arguments, "backup")?;
    let truncate_directory = args::boolean(arguments, "truncate_directory", false)?;
    let restore_startup = args::boolean(arguments, "restore_startup", false)?;

    request(
        panel,
        POST,
        &server_path(&server, &format!("/backups/{backup}/restore")),
        None,
        Some(&json!({
            "truncate_directory": truncate_directory,
            "restore_startup": restore_startup,
        })),
    )
    .await
}

async fn delete_backup(panel: &dyn PanelApi, arguments: &Map<String, Value>) -> ToolResult {
    let server = args::server(arguments, "server")?;
    let backup = args::server(arguments, "backup")?;

    request(
        panel,
        DELETE,
        &server_path(&server, &format!("/backups/{backup}")),
        None,
        None,
    )
    .await
}

async fn contents(panel: &dyn PanelApi, server: &str, file: &str) -> Result<Reply, String> {
    let mut query = Map::new();
    query.insert("file".to_string(), json!(file));

    panel
        .call(
            Method::GET,
            &server_path(server, "/files/contents"),
            Some(&query),
            None,
        )
        .await
}

async fn request(
    panel: &dyn PanelApi,
    method: &str,
    path: &str,
    query: Option<&Map<String, Value>>,
    body: Option<&Value>,
) -> ToolResult {
    let method = Method::from_bytes(method.as_bytes())
        .map_err(|err| format!("`method` is not a valid HTTP method: {err}"))?;

    let reply = panel.call(method, path, query, body).await?;
    if reply.status >= 400 {
        return Err(output::render(&reply));
    }

    Ok(output::render(&reply))
}

fn server_path(server: &str, suffix: &str) -> String {
    format!("/api/client/servers/{server}{suffix}")
}

fn parse_json(reply: &Reply, what: &str) -> Result<Value, String> {
    serde_json::from_slice(&reply.body)
        .map_err(|err| format!("failed to parse the {what} response: {err}"))
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_default()
}

fn decode_base64(encoded: &str) -> Result<Vec<u8>, String> {
    let encoded: Cow<str> = if encoded.chars().any(char::is_whitespace) {
        encoded
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .into()
    } else {
        encoded.into()
    };

    base64::engine::general_purpose::STANDARD
        .decode(encoded.as_ref())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded.as_ref()))
        .map_err(|err| format!("`content_base64` is not valid base64: {err}"))
}
