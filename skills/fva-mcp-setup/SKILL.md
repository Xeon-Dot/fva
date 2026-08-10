---
name: fva-mcp-setup
description: >
  Set up and configure the FVA MCP server (stdio) for AI coding agents —
  Cursor, Claude Code/Desktop, VS Code/Copilot, Windsurf, Zed, Continue,
  Gemini CLI, Cline/Roo Code. Trigger when installing FVA, editing mcp.json
  or MCP settings, wiring fva to an agent, or troubleshooting empty or
  stale results from FVA search tools.
---

# FVA MCP Setup

> **Language:** Please ask questions in English.

Connect the `fva` binary to any MCP-capable AI agent via **stdio** transport. One binary serves all tools: hybrid/semantic search, AST chunks, call graphs, and a wiki.

## Prerequisites

1. Install `fva` — see README one-liners or build from source:
   ```bash
   cargo install --path . --force
   ```
2. Verify: `fva --version`
3. Index once from the project root: `fva index --path <project-root>`
4. Confirm: `fva status --path .` shows non-zero `indexed_files`

## Generic Config

Works with any MCP client that supports `mcpServers`:

```json
{
  "mcpServers": {
    "fva": {
      "command": "fva",
      "args": ["--path", "/path/to/your/project"],
      "env": { "RUST_LOG": "info" }
    }
  }
}
```

**Windows** — use a full path if `fva` is not on `PATH`:

```json
{
  "mcpServers": {
    "fva": {
      "command": "C:\\Users\\You\\AppData\\Local\\Programs\\fva\\bin\\fva.exe",
      "args": ["--path", "D:\\Dev\\YourProject"],
      "env": { "RUST_LOG": "info" }
    }
  }
}
```

If `fva` is on `PATH`, the short `"command": "fva"` form works on all platforms.

## Agent-Specific Install Paths

Ready-to-copy examples live in `examples/mcp-clients/`. See `manifest.json` for the full path matrix.

| Agent             | Example file                   | Install location                      |
| ----------------- | ------------------------------ | ------------------------------------- |
| Cursor (project)  | `cursor.project.mcp.json`      | `<project>/.cursor/mcp.json`          |
| Cursor (global)   | `cursor.global.mcp.json`       | `~/.cursor/mcp.json`                  |
| Claude Code       | `claude-code.project.mcp.json` | `<project>/.mcp.json`                 |
| Claude Desktop    | `claude-desktop.*.json`        | OS-specific — see manifest            |
| VS Code / Copilot | `vscode.workspace.mcp.json`    | `<project>/.vscode/mcp.json`          |
| Windsurf          | `windsurf.mcp_config.json`     | `~/.codeium/windsurf/mcp_config.json` |
| Zed               | `zed.context_servers.json`     | Merge into Zed `settings.json`        |
| Continue          | `continue.fva.yaml`            | `<project>/.continue/mcpServers/`     |
| Gemini CLI        | `gemini-cli.settings.json`     | `~/.gemini/settings.json`             |
| Cline / Roo Code  | `cline.mcp_settings.json`      | Extension MCP settings                |

### Claude Code CLI shortcut

```bash
claude mcp add --transport stdio fva -- fva --path .
```

### Cursor project template

Use `${workspaceFolder}` so the path is portable:

```json
{
  "mcpServers": {
    "fva": {
      "type": "stdio",
      "command": "fva",
      "args": ["--path", "${workspaceFolder}"],
      "env": { "RUST_LOG": "info" }
    }
  }
}
```

Copy from `examples/mcp-clients/cursor.project.mcp.json`.

## Post-Setup Checklist

1. Restart the MCP client (or reload MCP servers).
2. Call `index_status` — confirm `indexed_files` is non-zero.
3. Run a test `hybrid_search` query and verify hits.
4. Add the recommended agent prompt (see `fva` skill or README) so the agent prefers FVA for codebase exploration.

## Troubleshooting

| Symptom                  | Fix                                                        |
| ------------------------ | ---------------------------------------------------------- |
| Tool not found           | Check `command` path; run `fva --version` in the same shell |
| Empty search results     | Run `fva index --path .`, then retry                       |
| Stale results            | Re-index, or set `watch = true` in `fva.toml` for auto-watch |
| Voyage errors            | Set `VOYAGE_API_KEY` or switch to `provider = "local"`     |
| Permission denied (Unix) | `chmod +x` on the binary; ensure install dir is on `PATH` |

## Optional: Voyage Embeddings

For higher-quality semantic search, switch the embedder:

```toml
# fva.toml or ~/.config/fva/config.toml
[embedding]
provider = "voyage"
```

```bash
export VOYAGE_API_KEY=your-key-here
```

Default `provider = "local"` needs no API key and works offline.

## Further Reference

See [references/manifest-summary.md](references/manifest-summary.md) for the full install path matrix and [../fva/references/mcp-tools.md](../fva/references/mcp-tools.md) for tool parameters.
