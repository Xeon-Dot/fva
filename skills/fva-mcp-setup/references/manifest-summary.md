# FVA MCP Client Install Paths

> **Language:** Please ask questions in English.

Source of truth: `examples/mcp-clients/manifest.json`. Copy the matching example file to the install path for your agent.

## Cursor

- **Project:** `<project>/.cursor/mcp.json` — example: `cursor.project.mcp.json` (use `${workspaceFolder}` for `--path`)
- **Project (Windows):** `<project>\.cursor\mcp.json` — example: `cursor.project.windows.mcp.json`
- **Global:** `~/.cursor/mcp.json` (Unix) or `%USERPROFILE%\.cursor\mcp.json` (Windows) — example: `cursor.global.mcp.json`

## Claude

- **Claude Code (project):** `<project>/.mcp.json` — example: `claude-code.project.mcp.json`
  CLI shortcut: `claude mcp add --transport stdio fva -- fva --path .`
- **Claude Desktop:**
  - macOS: `~/Library/Application Support/Claude/claude_desktop_config.json`
  - Linux: `~/.config/Claude/claude_desktop_config.json`
  - Windows: `%APPDATA%\Claude\claude_desktop_config.json`
  - Examples: `claude-desktop.macos-linux.json`, `claude-desktop.windows.json`

## VS Code / Copilot

- **Workspace:** `<project>/.vscode/mcp.json` — example: `vscode.workspace.mcp.json` (Windows variant: `vscode.workspace.windows.mcp.json`)
- **User:** MCP user configuration in the VS Code profile folder

## Windsurf

- Unix: `~/.codeium/windsurf/mcp_config.json` — example: `windsurf.mcp_config.json`
- Windows: `%USERPROFILE%\.codeium\windsurf\mcp_config.json` — example: `windsurf.windows.mcp_config.json`

## Zed

- Merge the `context_servers` block into Zed `settings.json` — example: `zed.context_servers.json` (Windows variant: `zed.windows.context_servers.json`)

## Continue

- `<project>/.continue/mcpServers/fva.yaml` — example: `continue.fva.yaml`

## Gemini CLI

- Merge the `mcpServers` block into `~/.gemini/settings.json` — example: `gemini-cli.settings.json`

## Cline / Roo Code

- Extension MCP settings (global storage or workspace) — examples: `cline.mcp_settings.json`, `roo-code.mcp_settings.json`

## Notes

- Replace `/path/to/your/project` with the actual project root. If `fva` is on `PATH`, `"command": "fva"` works on all platforms; on Windows without `PATH`, use the full `fva.exe` path.
- On Windows, prefer `.windows.json` variants or set the full path to `fva.exe`.
- Run `fva index --path .` once before heavy search workloads; confirm with `index_status` or `fva status --path .`.
- Set `VOYAGE_API_KEY` in env when using `provider = "voyage"` in `fva.toml` or `~/.config/fva/config.toml`.
