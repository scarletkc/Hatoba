# MCP servers

Settings → AI → **MCP Servers**: adding servers, **Always ask**, import and export, and each server's state and tools. The panel's **MCP Tools** menu is in `references/ai-panel.md`, how MCP calls ask in `references/ai-tools.md`, and errors in `references/troubleshooting-ai.md`.

MCP (Model Context Protocol) servers add tools to the assistant. Hatoba uses tools only (no prompts or resources). Two kinds: **Command** (stdio: a program started on this device with the user's privileges, which Hatoba talks to over standard input and output) and **HTTP** (a Streamable HTTP URL). A stdio server runs arbitrary code, so add only servers you trust. Tool descriptions reach the model as instructions.

| en | zh-CN | ja |
|---|---|---|
| Add Server | 添加服务器 | サーバーを追加 |
| Add from Template | 从模板添加 | テンプレートから追加 |
| Export | 导出 | エクスポート |
| Add MCP Server | 添加 MCP 服务器 | MCP サーバーを追加 |
| Type | 类型 | 種類 |
| Command | 命令 | コマンド |
| HTTP | HTTP | HTTP |
| Arguments | 参数 | 引数 |
| Split Into Arguments | 拆成参数 | 引数に分割 |
| Environment | 环境变量 | 環境変数 |
| URL | URL | URL |
| Headers | 请求头 | ヘッダー |
| Always ask | 始终询问 | 常に確認 |
| Import MCP Servers | 导入 MCP 服务器 | MCP サーバーをインポート |
| Preview | 预览 | プレビュー |

- **Add Server**: give it a **Name** and choose the **Type**. For **Command**: the program (looked up on `PATH` like a shell does, so `npx` finds `npx.cmd` on Windows), the **Arguments** (one per row; if a whole command line is pasted into the command, **Split Into Arguments** separates them) and **Environment** variables. For **HTTP**: the **URL** (HTTPS or HTTP) and **Headers** such as `Authorization`. Environment and header values are stored encrypted, can't be viewed after saving, and only the program or the server receives them.
- **Add from Template** fills in **Add MCP Server** for GitHub, Linear, Context7 or Cloudflare (HTTP servers that take a token, built into the app): the name, the URL and the `Authorization` header, with a link under it to where the token is created. Paste only the token: an `Authorization` value without a space is saved with `Bearer ` in front of it (for any HTTP server, also when it replaces a saved value). **Always ask** starts off. The saved server is an ordinary one.
- **Always ask** makes this server's tools ask before every call, even in Bypass mode. It syncs with the server.
- **Import** reads pasted JSON or a JSON file in the `mcpServers` format of Claude Desktop, Claude Code and Cursor (or VS Code's `servers` key), shows a **Preview**, and imports; values move into the vault. **Export** writes the same format with placeholders instead of the secret values.
- The server list syncs. Whether a server is on, and any **Always allow**, stay on each device. A server added on another device arrives on if it is HTTP and off if it is a Command. Editing a server, here or on another device, keeps whether it is on and its **Always allow**, except that an HTTP server changed into a Command on another device turns off here, as a new Command server arrives.
- A Command server starts when a conversation first needs its tools and stops when the app quits or the vault locks. Click a row to see its state, start, restart or stop it, read the error with the last lines of stderr, and see its tools with their hints (Read-only, Destructive, Idempotent, Open-world). Hints never change whether a call asks.

| en | zh-CN | ja |
|---|---|---|
| Stopped | 已停止 | 停止中 |
| Starting… | 正在启动… | 起動中… |
| Running | 运行中 | 実行中 |
| Failed | 失败 | 失敗 |
| Off on this device | 此设备上已关闭 | このデバイスではオフ |
| Start | 启动 | 起動 |
| Restart | 重新启动 | 再起動 |
| Stop | 停止 | 停止 |
| Always allow all tools | 始终允许所有工具 | すべてのツールを常に許可 |
| Always allow | 始终允许 | 常に許可 |
| Tools | 工具 | ツール |
| Last lines of stderr | stderr 的最后几行 | stderr の最後の行 |

- In manual mode every MCP tool asks for approval. **Always allow all tools** (per server) or **Always allow** (per tool) lets calls run without asking on this device. OAuth sign-in for MCP servers is not supported.

