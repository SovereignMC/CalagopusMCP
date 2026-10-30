use serde_json::{Value, json};

fn schema(properties: Value, required: Value) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}

fn string(description: &str) -> Value {
    json!({ "type": "string", "description": description })
}

fn boolean(description: &str, default: bool) -> Value {
    json!({ "type": "boolean", "description": description, "default": default })
}

fn integer(description: &str, default: Option<i64>, min: i64, max: i64) -> Value {
    let mut property = json!({
        "type": "integer",
        "description": description,
        "minimum": min,
        "maximum": max,
    });

    if let Some(default) = default {
        property["default"] = json!(default);
    }

    property
}

fn choice(description: &str, values: &[&str], default: Option<&str>) -> Value {
    let mut property = json!({
        "type": "string",
        "description": description,
        "enum": values,
    });

    if let Some(default) = default {
        property["default"] = json!(default);
    }

    property
}

fn string_list(description: &str) -> Value {
    json!({
        "type": "array",
        "description": description,
        "items": { "type": "string" },
    })
}

fn object(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "additionalProperties": true,
    })
}

fn tool(
    name: &str,
    title: &str,
    description: &str,
    input_schema: Value,
    read_only: bool,
    destructive: bool,
    idempotent: bool,
) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": read_only,
            "destructiveHint": destructive,
            "idempotentHint": idempotent,
            "openWorldHint": false,
        },
    })
}

const SERVER: &str = "Server identifier: the server's uuid or its short identifier.";

pub fn definitions() -> Vec<Value> {
    vec![
        tool(
            "search_api",
            "Search panel API",
            "Search every panel API endpoint, client and admin, by path, summary, description, \
             operation id or tag. Returns one line per match, such as `GET /api/client/servers — \
             List servers (query: page, per_page, search)`. Use this for anything the dedicated \
             tools do not cover, then `describe_api` and `read_api`/`write_api`.",
            schema(
                json!({
                    "query": string("Words to look for, for example `schedules create` or `user activity`."),
                    "scope": choice("Which endpoints to search.", &["all", "client", "admin"], Some("all")),
                    "limit": integer("Maximum number of matches to return.", Some(15), 1, 50),
                }),
                json!(["query"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "describe_api",
            "Describe panel API operation",
            "Describe one panel API operation: its parameters, request body, responses and the \
             schemas they reference. Call this before `read_api` or `write_api`, using a path \
             returned by `search_api`.",
            schema(
                json!({
                    "method": choice("HTTP method of the operation.", &["GET", "POST", "PUT", "PATCH", "DELETE"], None),
                    "path": string("Operation path exactly as `search_api` returned it, placeholders included, e.g. `/api/client/servers/{server}/schedules`."),
                }),
                json!(["method", "path"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "read_api",
            "Read panel API",
            "Send a GET to any panel client or admin API path, with the permissions of the \
             connected API key. Replace every `{placeholder}` in the path with a real value and \
             pass query parameters through `query`. `/api/admin` paths need admin permissions.",
            schema(
                json!({
                    "path": string("Concrete request path, for example `/api/client/servers/12345678/activity`."),
                    "query": object("Query parameters. Values may be strings, numbers, booleans or lists of those."),
                }),
                json!(["path"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "write_api",
            "Write panel API",
            "Send a mutating request to any panel client or admin API path, with the permissions \
             of the connected API key. Replace every `{placeholder}` in the path with a real \
             value and pass query parameters through `query`. A string `body` is sent as raw text \
             (use this for file contents); any other `body` is sent as JSON.",
            schema(
                json!({
                    "method": choice("HTTP method to use.", &["POST", "PUT", "PATCH", "DELETE"], None),
                    "path": string("Concrete request path, placeholders replaced."),
                    "query": object("Query parameters. Values may be strings, numbers, booleans or lists of those."),
                    "body": json!({
                        "description": "JSON request body, or a plain string to send as raw text.",
                    }),
                }),
                json!(["method", "path"]),
            ),
            false,
            true,
            false,
        ),
        tool(
            "list_servers",
            "List servers",
            "List the servers this API key can see, with paging and an optional search.",
            schema(
                json!({
                    "search": string("Search by server name or identifier."),
                    "page": integer("Page number.", Some(1), 1, super::args::MAX_PAGE),
                    "per_page": integer("Servers per page.", Some(25), 1, 100),
                    "other": boolean("Also list servers owned by other accounts that share them with this one.", false),
                }),
                json!([]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "get_server",
            "Get server",
            "Get one server's details together with its live resource usage: state, cpu, memory, \
             disk and network.",
            schema(
                json!({
                    "server": string(SERVER),
                }),
                json!(["server"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "power_server",
            "Power server",
            "Send a power action to a server. Needs `control.start`, `control.stop` or \
             `control.restart` on the API key, depending on the action.",
            schema(
                json!({
                    "server": string(SERVER),
                    "action": choice("Power action.", &["start", "stop", "restart", "kill"], None),
                }),
                json!(["server", "action"]),
            ),
            false,
            true,
            false,
        ),
        tool(
            "send_command",
            "Send console command",
            "Send one command to a running server's console. Needs `control.console`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "command": json!({
                        "type": "string",
                        "description": "Command to run, without a leading slash.",
                        "minLength": 1,
                        "maxLength": 1024,
                    }),
                }),
                json!(["server", "command"]),
            ),
            false,
            true,
            false,
        ),
        tool(
            "read_console",
            "Read console output",
            "Read recent console output for a server, with ANSI escapes stripped. Needs \
             `control.read-console`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "lines": integer("How many lines to read from the end.", Some(100), 1, 1000),
                }),
                json!(["server"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "list_files",
            "List files",
            "List one directory of a server's filesystem. Needs `files.read`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "directory": string("Directory to list, absolute inside the server. Defaults to `/`."),
                    "page": integer("Page number.", Some(1), 1, super::args::MAX_PAGE),
                    "per_page": integer("Entries per page.", Some(100), 1, 100),
                    "sort": choice(
                        "Sort order.",
                        &[
                            "name_asc", "name_desc", "size_asc", "size_desc", "physical_size_asc",
                            "physical_size_desc", "modified_asc", "modified_desc", "created_asc",
                            "created_desc",
                        ],
                        Some("name_asc"),
                    ),
                }),
                json!(["server"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "read_file",
            "Read file",
            "Read a text file from a server. Needs `files.read-content`. Files above 100000 bytes \
             are truncated with a note; files that are not UTF-8 report their size and point at \
             `download_file`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "file": string("Absolute path inside the server, for example `/server.properties`."),
                }),
                json!(["server", "file"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "download_file",
            "Download file",
            "Download any file from a server. Small UTF-8 files come back as text, everything else \
             as complete base64 that is never truncated. Files larger than `max_bytes` fail with \
             their size; use `pull_file` to move very large files around.",
            schema(
                json!({
                    "server": string(SERVER),
                    "file": string("Absolute path inside the server."),
                    "max_bytes": integer("Largest file to download, in bytes.", Some(4194304), 1, 33554432),
                }),
                json!(["server", "file"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "write_file",
            "Write file",
            "Write a text file on a server, replacing its contents. Needs `files.update`; the \
             parent directory must already exist.",
            schema(
                json!({
                    "server": string(SERVER),
                    "file": string("Absolute path inside the server."),
                    "content": string("Complete new file contents."),
                }),
                json!(["server", "file", "content"]),
            ),
            false,
            true,
            true,
        ),
        tool(
            "upload_file",
            "Upload file",
            "Upload a file from base64 content, up to 32 MiB decoded, into a server directory. \
             Needs `files.create`. For larger files use `pull_file` so the node downloads it \
             directly.",
            schema(
                json!({
                    "server": string(SERVER),
                    "directory": string("Destination directory, absolute inside the server. Defaults to `/`."),
                    "file": string("Destination file name, without a directory separator."),
                    "content_base64": string("File contents, base64 encoded (standard alphabet)."),
                }),
                json!(["server", "file", "content_base64"]),
            ),
            false,
            false,
            false,
        ),
        tool(
            "delete_files",
            "Delete files",
            "Delete files or directories on a server. They go to the server's trash unless \
             `permanent` is true. Needs `files.delete`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "root": string("Directory the names are relative to. Defaults to `/`."),
                    "files": string_list("Names of the files or directories to delete."),
                    "permanent": boolean("Delete outright instead of moving to the trash.", false),
                }),
                json!(["server", "files"]),
            ),
            false,
            true,
            false,
        ),
        tool(
            "rename_file",
            "Rename file",
            "Rename or move a file or directory on a server. Needs `files.update`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "root": string("Directory both paths are relative to. Defaults to `/`."),
                    "from": string("Current name, relative to `root`."),
                    "to": string("New name, relative to `root`."),
                }),
                json!(["server", "from", "to"]),
            ),
            false,
            true,
            false,
        ),
        tool(
            "create_directory",
            "Create directory",
            "Create a directory on a server. Needs `files.create`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "root": string("Parent directory. Defaults to `/`."),
                    "name": string("Directory name, relative to `root`."),
                }),
                json!(["server", "name"]),
            ),
            false,
            false,
            true,
        ),
        tool(
            "pull_file",
            "Pull file from url",
            "Have the node download a file from a url into the server, which is the right tool for \
             large files. Needs `files.create`. By default the download runs in the background.",
            schema(
                json!({
                    "server": string(SERVER),
                    "root": string("Destination directory. Defaults to `/`."),
                    "url": string("Url to download from."),
                    "name": string("Destination file name. Defaults to the name in the url."),
                    "use_header": boolean("Send the panel's authorization header to the source.", false),
                    "foreground": boolean("Wait for the download to finish instead of returning immediately.", false),
                }),
                json!(["server", "url"]),
            ),
            false,
            false,
            false,
        ),
        tool(
            "list_backups",
            "List backups",
            "List a server's backups. Needs `backups.read`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "search": string("Search by backup name or uuid."),
                    "page": integer("Page number.", Some(1), 1, super::args::MAX_PAGE),
                    "per_page": integer("Backups per page.", Some(10), 1, 100),
                    "ungrouped": boolean("Only list backups that are not in a backup group.", false),
                }),
                json!(["server"]),
            ),
            true,
            false,
            true,
        ),
        tool(
            "create_backup",
            "Create backup",
            "Start a backup of a server. Needs `backups.create`. `ignored_files` leaves matching \
             files out of the archive.",
            schema(
                json!({
                    "server": string(SERVER),
                    "name": string("Backup name. The panel picks one when omitted."),
                    "ignored_files": string_list("Files to leave out of the backup."),
                }),
                json!(["server"]),
            ),
            false,
            false,
            false,
        ),
        tool(
            "restore_backup",
            "Restore backup",
            "Restore a backup over a server's files. Needs `backups.restore`. This is destructive: \
             it overwrites what is on the server now, and with `truncate_directory` it deletes the \
             server's files first.",
            schema(
                json!({
                    "server": string(SERVER),
                    "backup": string("Backup uuid."),
                    "truncate_directory": boolean("Delete the server's files before restoring.", false),
                    "restore_startup": boolean("Also restore the startup command and variables saved in the backup.", false),
                }),
                json!(["server", "backup"]),
            ),
            false,
            true,
            false,
        ),
        tool(
            "delete_backup",
            "Delete backup",
            "Delete a backup permanently. Needs `backups.delete`.",
            schema(
                json!({
                    "server": string(SERVER),
                    "backup": string("Backup uuid."),
                }),
                json!(["server", "backup"]),
            ),
            false,
            true,
            true,
        ),
    ]
}
