#!/usr/bin/env bash
# Live end-to-end check of the Calagopus MCP extension against a real panel.
#
#   PANEL_URL=https://panel.example.com \
#   API_KEY='<panel api key>' \
#   SERVER_ID='<uuid or short identifier of a server the key controls>' \
#   ./scripts/smoke.sh
#
# The key needs server permissions control.start, control.stop, control.restart,
# control.read-console, files.read, files.read-content, files.create, files.update and
# files.delete. The script starts and stops the server and writes then deletes a file.
set -euo pipefail

: "${PANEL_URL:?set PANEL_URL, for example https://panel.example.com}"
: "${API_KEY:?set API_KEY to a panel api key}"
: "${SERVER_ID:?set SERVER_ID to a server the key can control}"

PACKAGE='dev_sovereignmc_mcpserver'
ENDPOINT="${PANEL_URL%/}/api/client/extensions/${PACKAGE}/mcp"

TOOLS=(
  search_api describe_api read_api write_api
  list_servers get_server power_server send_command read_console
  list_files read_file download_file write_file upload_file delete_files rename_file
  create_directory pull_file
  list_backups create_backup restore_backup delete_backup
)

fail() {
  printf '\nFAIL: %s\n' "$1" >&2
  if [ "${2:-}" != "" ]; then
    printf 'response:\n%s\n' "$2" >&2
  fi
  exit 1
}

ok() {
  printf 'ok - %s\n' "$1"
}

rpc() {
  curl -sS --fail-with-body -X POST "$ENDPOINT" \
    -H "Authorization: Bearer ${API_KEY}" \
    -H 'Content-Type: application/json' \
    -H 'Accept: application/json' \
    --data-binary "$1"
}

expect() {
  local description=$1 body=$2 filter=$3
  printf '%s' "$body" | jq -e "$filter" >/dev/null 2>&1 || fail "$description" "$body"
  ok "$description"
}

tool() {
  local id=$1 name=$2 arguments=$3 payload response
  payload=$(jq -nc --argjson id "$id" --arg name "$name" --argjson arguments "$arguments" \
    '{jsonrpc: "2.0", id: $id, method: "tools/call", params: {name: $name, arguments: $arguments}}')
  response=$(rpc "$payload") || fail "tools/call ${name} did not answer with http 200" "$response"

  printf '%s' "$response" | jq -e '.error' >/dev/null 2>&1 \
    && fail "tools/call ${name} returned a json-rpc error" "$response"
  printf '%s' "$response" | jq -e '.result.isError == true' >/dev/null 2>&1 \
    && fail "tools/call ${name} reported a tool error" "$response"

  printf '%s' "$response" | jq -r '.result.content[0].text'
}

server_arguments() {
  jq -nc --arg server "$SERVER_ID" '{server: $server}'
}

printf 'panel:    %s\n' "$PANEL_URL"
printf 'endpoint: %s\n' "$ENDPOINT"
printf 'server:   %s\n\n' "$SERVER_ID"

# 1. initialize ---------------------------------------------------------------
initialize=$(rpc '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"calagopus-mcp-smoke","version":"0.0.0"}}}') \
  || fail "initialize did not answer with http 200" "$initialize"
expect "initialize negotiates 2025-06-18" "$initialize" '.result.protocolVersion == "2025-06-18"'
expect "initialize identifies the server" "$initialize" '.result.serverInfo.name == "calagopus-mcp"'

# 2. tools/list ---------------------------------------------------------------
catalogue=$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list"}') \
  || fail "tools/list did not answer with http 200" "$catalogue"

for name in "${TOOLS[@]}"; do
  printf '%s' "$catalogue" | jq -e --arg name "$name" 'any(.result.tools[]; .name == $name)' >/dev/null 2>&1 \
    || fail "tools/list is missing ${name}" "$catalogue"
done
ok "tools/list exposes all ${#TOOLS[@]} tools"

# 3. list_servers -------------------------------------------------------------
servers=$(tool 3 list_servers '{"per_page":100}') || fail "list_servers failed" "$servers"
printf '%s' "$servers" | jq -e 'has("servers")' >/dev/null 2>&1 || fail "list_servers returned no servers key" "$servers"
ok "list_servers returns a servers page"

# 3b. meta tools over the panel openapi document -----------------------------
matches=$(tool 31 search_api '{"query":"servers","scope":"client","limit":5}') \
  || fail "search_api failed" "$matches"
printf '%s' "$matches" | grep -q 'GET /api/client/servers' \
  || fail "search_api did not find GET /api/client/servers" "$matches"
ok "search_api finds panel operations"

described=$(tool 32 describe_api '{"method":"GET","path":"/api/client/servers/{server}"}') \
  || fail "describe_api failed" "$described"
printf '%s' "$described" | jq -e '.path == "/api/client/servers/{server}"' >/dev/null 2>&1 \
  || fail "describe_api did not return the operation" "$described"
printf '%s' "$described" | jq -e '.components.schemas | type == "object"' >/dev/null 2>&1 \
  || fail "describe_api returned no schemas" "$described"
ok "describe_api describes an operation with its schemas"

read=$(tool 33 read_api "$(jq -nc --argjson query '{"per_page":5}' '{path: "/api/client/servers", query: $query}')") \
  || fail "read_api failed" "$read"
printf '%s' "$read" | jq -e 'has("servers")' >/dev/null 2>&1 || fail "read_api returned no servers key" "$read"
ok "read_api reads an arbitrary panel endpoint"

# 4. get_server ---------------------------------------------------------------
details=$(tool 4 get_server "$(server_arguments)") || fail "get_server failed" "$details"
printf '%s' "$details" | jq -e --arg server "$SERVER_ID" '.server.uuid == $server' >/dev/null 2>&1 \
  || fail "get_server did not return server ${SERVER_ID}" "$details"
printf '%s' "$details" | jq -e '.resources | type == "object"' >/dev/null 2>&1 \
  || fail "get_server returned no resources object" "$details"
ok "get_server returns the server and its resources"

# 5. power start and wait for running ----------------------------------------
powered=$(tool 5 power_server "$(jq -nc --arg server "$SERVER_ID" '{server: $server, action: "start"}')") \
  || fail "power_server start failed" "$powered"
ok "power_server start accepted"

deadline=$((SECONDS + 120))
state=''
while :; do
  details=$(tool 4 get_server "$(server_arguments)") || fail "get_server failed while polling" "$details"
  state=$(printf '%s' "$details" | jq -r '.resources.state // "unknown"')
  [ "$state" = 'running' ] && break
  [ "$SECONDS" -ge "$deadline" ] && fail "server did not reach running within 120s (last state: ${state})" "$details"
  sleep 3
done
ok "server reached running"

# 6. upload, read back and delete a file --------------------------------------
stamp="smoke-$(date +%s)"
content="calagopus-mcp ${stamp}"
encoded=$(printf '%s' "$content" | base64 | tr -d '\n')

uploaded=$(tool 6 upload_file "$(jq -nc --arg server "$SERVER_ID" --arg encoded "$encoded" \
  '{server: $server, directory: "/", file: "calagopus-mcp-smoke.txt", content_base64: $encoded}')") \
  || fail "upload_file failed" "$uploaded"
ok "upload_file wrote /calagopus-mcp-smoke.txt"

readback=$(tool 6 read_file "$(jq -nc --arg server "$SERVER_ID" '{server: $server, file: "/calagopus-mcp-smoke.txt"}')") \
  || fail "read_file failed" "$readback"
[ "$readback" = "$content" ] || fail "read_file did not return the uploaded text (got: ${readback})" ""
ok "read_file returned the uploaded text"

deleted=$(tool 6 delete_files "$(jq -nc --arg server "$SERVER_ID" \
  '{server: $server, root: "/", files: ["calagopus-mcp-smoke.txt"], permanent: true}')") \
  || fail "delete_files failed" "$deleted"
ok "delete_files removed /calagopus-mcp-smoke.txt"

# 7. power stop ---------------------------------------------------------------
stopped=$(tool 7 power_server "$(jq -nc --arg server "$SERVER_ID" '{server: $server, action: "stop"}')") \
  || fail "power_server stop failed" "$stopped"
ok "power_server stop accepted"

printf '\nall checks passed\n'
