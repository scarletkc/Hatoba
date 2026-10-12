# AI assistant panel

The AI assistant is a chat panel at the right edge of the window, below the tab bar. It can read the terminal, run commands, type into the shell, search the web, fetch pages, read skills and use MCP tools, within the permission mode of the conversation. It calls a model provider that the user adds with their own API key (Settings → AI, see `references/ai-settings.md`).

## Opening the panel

- Click the sparkle button at the right of the tab bar, or press Ctrl+Shift+A (⌘⇧A on macOS). The close button (×) in the panel header hides it again. The open state and the width are remembered on this device.
- **Ask AI** in a terminal's right-click menu or **More actions** menu opens the panel on that tab with the input focused; the selected text shows as a chip above the input. It is disabled while nothing is selected, and the right-click menu appears only when **Right-Click in Terminal** is set to **Show context menu** (`references/terminal.md`). **Ask AI** on a failed connection's card attaches its diagnostics.
- Drag the panel's left edge to resize it (300 to 900 px); double-click the edge to reset the width ("Drag to resize, double-click to reset").
- With no provider, the panel says "Add a model provider" with a button that opens Settings → AI. With a provider that has no model, it asks to add a model.

| en | zh-CN | ja |
|---|---|---|
| AI Assistant | AI 助手 | AI アシスタント |
| Show AI Panel | 显示 AI 面板 | AI パネルを表示 |
| Hide AI Panel | 隐藏 AI 面板 | AI パネルを隠す |
| Close AI Panel | 关闭 AI 面板 | AI パネルを閉じる |
| Open Settings → AI | 打开设置 → AI | 設定 → AI を開く |
| Ask AI | 询问 AI | AI に質問 |
| More actions | 更多操作 | その他の操作 |
| Right-Click in Terminal | 终端中的右键 | ターミナルでの右クリック |
| Show context menu | 弹出菜单 | コンテキストメニューを表示 |

## One conversation per tab

The panel shows the conversation of the active tab, and switching tabs switches the panel. Every terminal tab has its own conversation, and the home tab (host list, Keys, Cloud Sync) has a chat-only one. A tab shows a small sparkle while its assistant is working and a hand while it waits for your approval.

- Header: the conversation title, the tab's host, **History**, **New Conversation**, and close.
- Input area: attachment chips, a text box (Enter sends, Shift+Enter adds a line; nothing is sent while an input method is composing), under it on the left the + button (**Add Files and More**, a menu with **Add Text Files…**, **Send Terminal Screen** and **MCP Tools**) and the permission mode button, and on the right the context meter, the **Model** picker, **Ask a Side Question**, and **Send (Enter)** (replaced by **Stop** while a turn runs). A second message cannot be sent while the first is still starting (during its compaction, say).
- Messages: each of yours shows the time you sent it, and each reply, once it is finished, the time it ended (hover for the full date and time). **Copy** next to one of your messages copies what you typed, without the attachments; **Copy** under a reply copies its Markdown, without the reasoning and tool calls, and a reply that used tools is copied whole, with its text from before and after the calls. **Fork from Here** beside them copies the conversation up to that point into a new one (`references/ai-history.md`).
- A conversation acts only on its own tab, never on another, in either permission mode. While the tab is disconnected or still connecting, the terminal tools are unavailable (the panel says so and offers **Reconnect**). Closing a tab detaches its conversation; it stays in History.
- Opening a conversation from History attaches it to the active tab. If that tab's host differs, the panel says so, and the next message moves the conversation to the tab's host. That message gets a divider above it, "Moved to <new host> (was on <old host>)", and the assistant is told that the screens and command output before it came from the old host. A conversation that is running a turn, waiting for approval or compacting in another tab is not moved: that tab is brought to the front instead, because its turn acts on that tab's terminal. To continue on another host and leave the original where it is, fork it instead.
- A quick connection counts as a host named by its `user@host:port`, in the panel and in History, so a conversation that moves from one to a saved host or to another quick connection gets the same notice and divider. Saving a quick connection as a host (**Save as Host…**) keeps the same server, so a conversation that continues on the saved host gets neither.
- With no terminal tab (the home tab), or with a tab that is not connected, the assistant can still chat, search the web (when a search provider is set), fetch pages, read skills and use MCP tools. It cannot read the terminal, run commands, send input, or read, edit or write files: those tools are not offered, and a call to one anyway returns an error without asking. For a conversation that belongs to a host or a quick connection, the panel offers a "Connect to <host>" button that opens a tab and attaches the conversation; a quick connection saved as a host since opens as that host.
- A new terminal tab starts with its own empty conversation, and the home tab's chat stays on the home tab. While the tab's conversation has no messages and the home tab's chat has some and is not working, waiting for approval or compacting, the panel offers **Continue Here**, which moves that chat into the tab and gives the home tab a new conversation. Whatever the tab's input holds stays, a selection or diagnostics chip included; when it holds nothing, the home tab's unsent text and attachments come along. The next message moves the chat to the tab's host.
- When the terminal tools come or go between two messages, as when a home tab chat continues in a connected tab or a tab disconnects and connects again, the next message gets a divider above it, "Terminal now available" or "Terminal no longer available", and the assistant is told that its earlier replies had the tools of their time.

| en | zh-CN | ja |
|---|---|---|
| New Conversation | 新对话 | 新しい会話 |
| History | 历史对话 | 履歴 |
| Model | 模型 | モデル |
| Thinking Level | 思考程度 | 思考レベル |
| Compact | 压缩 | 圧縮 |
| Send (Enter) | 发送（Enter） | 送信（Enter） |
| Stop | 停止 | 停止 |
| Add Files and More | 添加文件等 | ファイルなどを追加 |
| MCP Tools | MCP 工具 | MCP ツール |
| The assistant is working | AI 助手正在工作 | アシスタントが作業中です |
| The assistant is waiting for you | AI 助手在等你确认 | アシスタントが確認を待っています |
| Reconnect | 重新连接 | 再接続 |
| Continue Here | 在这里继续 | ここで続ける |

## Tools and approvals

The tools (read the terminal, run commands, type into the shell, read, edit and write files, search the web, fetch pages, read skills, MCP tools), the permission modes (**Manual approval**, **Bypass**) and the approval card (**Run**, **Edit**, **Reject**, **Allow for This Conversation**, **Always Allow**) are in `references/ai-tools.md`.

## Attachments

What goes with a message is covered in `references/ai-attachments.md`: the terminal's screen, the terminal selection, **Ask AI** about a failed connection (diagnostics), long pastes (Ctrl+Shift+V pastes as text), and text files attached from the + menu, the clipboard or a drop on the panel.

## Model, thinking level, context and compaction

The **Model** picker (with its **Thinking Level** row), the context meter, **Compact**, automatic compaction and the reasoning display are in `references/ai-models.md`. Provider and model setup is in `references/ai-settings.md`.

## History, edit and fork

**History** (search, open, rename, pin, export, delete), **Edit and Resend**, and forking a conversation (**Fork from Here**, **Fork**) are in `references/ai-history.md`.

## MCP tools menu

**MCP Tools** in the input area's + menu shows, in place of the menu, the MCP servers that are on for this device, with their state, and a switch for each; the back arrow (or ←) returns to the menu. The item shows how many of them this conversation uses, such as 2/3, and a red dot marks it and the + button while a server has an error. Switching one off removes its tools from this conversation until the app quits, and stops its calls in a running turn too: they return an error result. **Manage in Settings → AI** opens the settings. Adding, editing and enabling servers is in `references/ai-settings.md`.

Once a conversation's MCP tool definitions would take a tenth of the model's context window (3,200 tokens when it is not set), or a request would carry over 128 tools in all, they stop going with each request from then on, which keeps the provider's prompt cache working. A message lists the tools by name instead, shown as a divider "Listed N MCP tools" (列出了 N 个 MCP 工具, MCP ツールを N 個一覧にしました). The assistant calls a listed tool directly, or finds one with **Search MCP tools**.

| en | zh-CN | ja |
|---|---|---|
| MCP servers for this conversation | 此对话使用的 MCP 服务器 | この会話で使う MCP サーバー |
| Manage in Settings → AI | 在设置 → AI 中管理 | 設定 → AI で管理 |
| Off for this conversation | 此对话中已关闭 | この会話ではオフ |
| Starts when needed | 需要时启动 | 必要なときに起動 |
| Starting… | 正在启动… | 起動しています… |
| Failed | 出错 | エラー |
| Starting MCP servers… | 正在启动 MCP 服务器… | MCP サーバーを起動しています… |

Provider and tool errors in the panel are explained in `references/troubleshooting-ai.md`.
