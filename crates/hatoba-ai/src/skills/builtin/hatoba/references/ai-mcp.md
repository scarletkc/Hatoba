# MCP servers

Settings → AI → **MCP Servers**: adding servers, **Always ask**, import and export, and each server's state and tools. The panel's **MCP Tools** and `/mcp`, MCP prompts as `/` commands and MCP resources under `@` are in `references/ai-panel.md`, how MCP calls ask in `references/ai-tools.md`, and errors in `references/troubleshooting-ai.md`.

MCP (Model Context Protocol) servers add tools to the assistant, and prompts and resources that you attach yourself: a prompt runs as a `/` command, and a resource is picked after `@`. Resource templates are not listed. Two kinds: **Command** (stdio: a program started on this device with the user's privileges, which Hatoba talks to over standard input and output) and **HTTP** (a Streamable HTTP URL). A stdio server runs arbitrary code, so add only servers you trust. Tool descriptions reach the model as instructions.

Other labels: **Import MCP Servers**.

- **Add Server**: give it a **Name** and choose the **Type**. For **Command**: the program (looked up on `PATH` like a shell does, so `npx` finds `npx.cmd` on Windows), the **Arguments** (one per row; if a whole command line is pasted into the command, **Split Into Arguments** separates them) and **Environment** variables. For **HTTP**: the **URL** (HTTPS or HTTP) and **Headers** such as `Authorization`. Environment and header values are stored encrypted, can't be viewed after saving, and only the program or the server receives them.
- **Add from Template** fills in **Add MCP Server** for GitHub, Linear, Context7 or Cloudflare (HTTP servers that take a token, built into the app): the name, the URL and the `Authorization` header, with a link under it to where the token is created. Paste only the token: an `Authorization` value without a space is saved with `Bearer ` in front of it (for any HTTP server, also when it replaces a saved value). **Always ask** starts off. The saved server is an ordinary one.
- **Always ask** makes this server's tools ask before every call, even in Bypass mode. It syncs with the server.
- **Import** reads pasted JSON or a JSON file in the `mcpServers` format of Claude Desktop, Claude Code and Cursor (or VS Code's `servers` key), shows a **Preview**, and imports; values move into the vault. **Export** writes the same format with placeholders instead of the secret values.
- The server list syncs. Whether a server is on, and any **Always allow**, stay on each device. A server added on another device arrives on if it is HTTP and off if it is a Command. Editing a server, here or on another device, keeps whether it is on and its **Always allow**, except that an HTTP server changed into a Command on another device turns off here, as a new Command server arrives.
- A Command server starts when a conversation first needs its tools and stops when the app quits or the vault locks. Click a row to see its state, start, restart or stop it, read the error with the last lines of stderr, and see its tools with their hints (Read-only, Destructive, Idempotent, Open-world). Hints never change whether a call asks.

Other labels: **Stopped**, **Starting…**, **Running**, **Failed**, **Off on this device**, **Start**, **Restart**, **Stop**, **Last lines of stderr**.

- In manual mode every MCP tool asks for approval. **Always allow all tools** (per server) or **Always allow** (per tool) lets calls run without asking on this device. OAuth sign-in for MCP servers is not supported.
