# Calagopus MCP

A [Model Context Protocol](https://modelcontextprotocol.io) server that runs **inside** a Calagopus
Panel. An MCP client (Claude Code, Cursor, VS Code, Claude Desktop) connects with a panel API key and
gets the panel's server control, console, file and backup operations as tools, plus a meta-tool
layer that reaches every other panel API endpoint — client and admin.

`package_name = "dev.sovereignmc.mcpserver"`, requires panel `>=1.2.0`.

The package name has exactly three dot-separated segments, with no hyphens, because the panel
derives the install directory, the Cargo crate name and the module path from it by replacing `.`
with `_` only: `dev.sovereignmc.mcp-server` would produce `dev_sovereignmc_mcp-server`, which is
neither a valid Cargo package name nor a valid Rust path, and a name with four segments is rejected
by `is_valid_package_identifier` (it requires exactly three).

## How it works

The endpoint is mounted at `/api/client/extensions/dev_sovereignmc_mcpserver/mcp`, so the panel's
own client middleware authenticates the request before the MCP handler runs. Every tool call then
executes the panel's real HTTP route **in process** with the calling user's identity:

```
MCP client ──Bearer key──▶ /api/client/extensions/…/mcp
                                 │ panel client auth (rate limit, account gates, permissions)
                                 ▼
                           tool handler ──▶ send_authenticated_router_oneshot
                                 │            GET /api/client/servers/…
                                 ▼
                            panel route + permission check + activity log
```

Nothing is reimplemented: permission checks, API-key scoping, account gates, rate limits, activity
logging and every future route apply unchanged. A tool can never do more than the API key allows.

## Install

Extensions need the `heavy` panel image (`compose.heavy.yml` / `compose.heavy.aio.yml`); the minimal
image ships without the Rust toolchain and cannot build an extension. Building this extension needs
roughly 8 GB of free RAM on the panel host.

1. Build the archive from this repository:

   ```bash
   zip -r dev_sovereignmc_mcpserver.c7s.zip Metadata.toml backend frontend
   ```

2. In the panel: **Admin → Extensions → Upload**, pick the `.c7s.zip`, and wait for the build to
   finish.
3. Restart the panel.
4. Create an API key under **Account → API Keys** with the permissions you want the agent to have:

   | Area | Permissions for |
   |---|---|
   | Server list | `servers.read` — a **user** permission, needed by `list_servers` and by `read_api` on `/api/client/servers` (`other: true` also needs the admin permission of the same name) |
   | Server control | `control.start`, `control.stop`, `control.restart`, `control.console`, `control.read-console` |
   | Files | `files.read`, `files.read-content`, `files.create`, `files.update`, `files.delete` |
   | Backups | `backups.read`, `backups.create`, `backups.restore`, `backups.delete` |
   | Admin tools | whichever admin permissions the meta tools should reach |

   API keys are limited by the account's two-factor and email-verification state, exactly like the
   rest of the API.

## Connect a client

Endpoint: `https://<panel>/api/client/extensions/dev_sovereignmc_mcpserver/mcp`

Claude Code:

```bash
claude mcp add --transport http calagopus \
  https://panel.example.com/api/client/extensions/dev_sovereignmc_mcpserver/mcp \
  --header "Authorization: Bearer <key>"
```

Cursor (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "calagopus": {
      "url": "https://panel.example.com/api/client/extensions/dev_sovereignmc_mcpserver/mcp",
      "headers": { "Authorization": "Bearer <key>" }
    }
  }
}
```

VS Code (`.vscode/mcp.json`):

```json
{
  "servers": {
    "calagopus": {
      "type": "http",
      "url": "https://panel.example.com/api/client/extensions/dev_sovereignmc_mcpserver/mcp",
      "headers": { "Authorization": "Bearer <key>" }
    }
  }
}
```

Claude Desktop has no remote HTTP transport of its own, so bridge it with `mcp-remote`
(`claude_desktop_config.json`):

```json
{
  "mcpServers": {
    "calagopus": {
      "command": "npx",
      "args": [
        "-y",
        "mcp-remote",
        "https://panel.example.com/api/client/extensions/dev_sovereignmc_mcpserver/mcp",
        "--header",
        "Authorization: Bearer <key>"
      ]
    }
  }
}
```

## Tools

Server:

| Tool | What it does |
|---|---|
| `list_servers` | List visible servers, with search and paging |
| `get_server` | Server details merged with live resource usage (state, cpu, memory, disk, network) |
| `power_server` | `start`, `stop`, `restart`, `kill` |
| `send_command` | Send one console command |
| `read_console` | Tail recent console output, ANSI stripped |

Files:

| Tool | What it does |
|---|---|
| `list_files` | List a directory |
| `read_file` | Read a text file (100 000 byte cap with a note) |
| `download_file` | Any file: text when small and UTF-8, otherwise complete base64 |
| `write_file` | Write a text file |
| `upload_file` | Upload base64 content, up to 32 MiB decoded |
| `delete_files` | Move to trash, or delete permanently |
| `rename_file` | Rename or move |
| `create_directory` | Create a directory |
| `pull_file` | Have the node download a URL into the server (the right tool for large files) |

Backups: `list_backups`, `create_backup`, `restore_backup`, `delete_backup`.

Meta tools, for **every** other panel endpoint including all of `/api/admin`:

```
search_api   {"query": "schedules create", "scope": "client"}
  → GET /api/client/servers/{server}/schedules — List schedules (query: page, per_page)
describe_api {"method": "POST", "path": "/api/client/servers/{server}/schedules"}
read_api     {"path": "/api/client/servers/abc/schedules"}
write_api    {"method": "POST", "path": "/api/client/servers/abc/schedules",
              "body": {"name": "nightly", "cron": "0 4 * * *"}}
```

Replace every `{placeholder}` in a path before calling `read_api` or `write_api`, and send query
parameters through the `query` object rather than inside the path.

## Limits

- **API key only.** Browser sessions are refused with `403`: an MCP client has no cookie, and the
  endpoint must not be usable from a browser tab.
- **JSON responses only.** Stateless MCP over HTTP: `POST` returns `application/json`; `GET` returns
  `405` with `Allow: POST`. Clients that require an SSE stream are unsupported.
- **Protocol versions** `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05` are negotiated; an
  unknown request falls back to `2025-11-25`.
- **Sizes:** 64 MiB request bodies, 16 MiB panel responses, 100 000 byte text results (truncated with
  a note), 32 MiB decoded uploads, 4 MiB default `download_file` limit.
  Raise the reverse proxy's `client_max_body_size` to match if you upload files near the limit.
- **Uploads go through the node.** `upload_file` asks the panel for the node's upload URL and posts
  the multipart body there, so the **panel** must be able to reach the node's advertised URL. On a
  split-DNS or self-signed setup that fails; use `pull_file` or `write_file` instead.

## Development

The extension is built against a panel checkout, because it depends on the panel's workspace crates:

```bash
git clone --depth 1 https://github.com/calagopus/panel.git /tmp/calagopus-panel
PKG=dev_sovereignmc_mcpserver
DEST=/tmp/calagopus-panel/backend-extensions/$PKG
mkdir -p "$DEST"
cp Metadata.toml "$DEST/"
cp -r backend/. "$DEST/"

cd /tmp/calagopus-panel
SQLX_OFFLINE=true cargo test   -p dev_sovereignmc_mcpserver
SQLX_OFFLINE=true cargo clippy -p dev_sovereignmc_mcpserver -- -D warnings
```

`cp -r backend/.` mirrors what installing the archive does: `backend/Cargo.toml` and `backend/src`
become the crate root of `backend-extensions/<package_name>`.

`scripts/smoke.sh` runs the live end-to-end check against an installed panel:

```bash
PANEL_URL=https://panel.example.com API_KEY='<key>' SERVER_ID='<server>' ./scripts/smoke.sh
```

It initializes the session, checks the tool catalogue, lists servers, reads a server and its
resources, starts the server and waits for `running`, uploads and reads back a file, deletes it, and
stops the server.
