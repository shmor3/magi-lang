# MAGI MCP Server

MCP (Model Context Protocol) server exposing MAGI language tools to AI assistants.

## Start

```bash
magi mcp
```

Communicates over stdio using JSON-RPC with Content-Length headers.

## Tools

| Tool | Description | Parameters |
|------|-------------|------------|
| `magi_run` | Execute MAGI code, return output | `code: string` |
| `magi_check` | Type-check code, return diagnostics | `code: string` |
| `magi_format` | Format MAGI code | `code: string` |
| `magi_lint` | Lint code, return warnings | `code: string` |
| `magi_parse` | Parse code, return AST summary | `code: string` |
| `magi_stdlib` | List stdlib modules/functions | `module: string` (optional) |
| `magi_version` | Return version info | (none) |

## Claude Desktop Configuration

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "magi": {
      "command": "magi",
      "args": ["mcp"]
    }
  }
}
```

## Protocol

Implements MCP protocol version `2024-11-05`. Supports:
- `initialize` / `initialized`
- `tools/list`
- `tools/call`
- `shutdown`

## Example

```
-> {"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"magi_run","arguments":{"code":"output 1 + 2;"}}}
<- {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"3"}]}}
```
