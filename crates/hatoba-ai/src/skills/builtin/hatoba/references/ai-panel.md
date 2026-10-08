# AI assistant panel

The AI assistant is a chat panel at the right edge of the window, below the tab bar. It can read the terminal, run commands, type into the shell, search the web, fetch pages, read skills and use MCP tools, within the permission mode of the conversation. It calls a model provider that the user adds with their own API key (Settings → AI, see `references/ai-settings.md`). Hatoba runs no AI service.

## Opening the panel

- Click the sparkle button at the right of the tab bar, or press Ctrl+Shift+A (⌘⇧A on macOS). The close button (×) in the panel header hides it again. The open state and the width are remembered on this device.
- **Ask AI** in a terminal's right-click menu or **More actions** menu opens the panel on that tab with the input focused; the selected text shows as a chip above the input. It is disabled while nothing is selected, and the right-click menu appears only when **Right-Click in Terminal** is set to **Show context menu** (`references/terminal.md`). **Ask AI** on a failed connection's card is covered below.
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

- Header: the conversation title, the tab's host, the permission mode button, **History**, **New Conversation**, and close.
- Input area: a text box (Enter sends, Shift+Enter adds a line; nothing is sent while an input method is composing), the **Model** picker, the **MCP tools** button, the context meter, and **Send (Enter)** (replaced by **Stop** while a turn runs).
- A conversation acts only on its own tab, never on another, in either permission mode. While the tab is disconnected or still connecting, the terminal tools are unavailable (the panel says so and offers **Reconnect**). Closing a tab detaches its conversation; it stays in History.
- Opening a conversation from History attaches it to the active tab. If that tab's host differs, the panel says so, and the next message moves the conversation to the tab's host.
- With no terminal tab (the home tab), or with a tab that is not connected, the assistant can still chat, search the web (when a search provider is set), fetch pages, read skills and use MCP tools. It cannot read the terminal, run commands or send input: those three tools are not offered, and a call to one anyway returns an error without asking. For a conversation that belongs to a host, the panel offers a "Connect to <host>" button that opens a tab and attaches the conversation.

| en | zh-CN | ja |
|---|---|---|
| New Conversation | 新对话 | 新しい会話 |
| History | 历史对话 | 履歴 |
| Model | 模型 | モデル |
| Send (Enter) | 发送（Enter） | 送信（Enter） |
| Stop | 停止 | 停止 |
| MCP tools | MCP 工具 | MCP ツール |
| The assistant is working | AI 助手正在工作 | アシスタントが作業中です |
| The assistant is waiting for you | AI 助手在等你确认 | アシスタントが確認を待っています |
| Reconnect | 重新连接 | 再接続 |

## Tools and approvals

| en | zh-CN | ja |
|---|---|---|
| Read terminal | 读取终端 | ターミナルを読み取る |
| Run command | 运行命令 | コマンドを実行 |
| Type in terminal | 向终端输入 | ターミナルに入力 |
| Web search | 搜索网页 | Web 検索 |
| Fetch page | 读取网页 | ページを取得 |
| Read skill | 读取技能 | スキルを読む |

| Tool | What it does | Manual approval mode |
|---|---|---|
| **Read terminal** (`read_terminal`) | Returns the visible screen plus up to 100 lines of scrollback (at most 1,000) as text, and whether a full-screen program (vim, htop) is active | Runs without asking |
| **Run command** (`run_command`) | Runs a command on a new exec channel of the tab's SSH connection and returns stdout, stderr and the exit status. It has no PTY, does not share the shell's directory, environment or sudo session, and its output does not show in the terminal. Timeout 30 s by default, at most 600 s | Asks |
| **Type in terminal** (`send_input`) | Types text into the shell as the keyboard would, optionally followed by Enter, Tab, Esc, Ctrl+C, Ctrl+D or an arrow key, then waits for output (10 s by default, at most 120 s). A command started this way keeps running if the turn is stopped | Asks |
| **Web search** (`web_search`) | Searches with the search provider chosen in Settings → AI. Not offered when none is chosen | Runs without asking |
| **Fetch page** (`fetch_url`) | Fetches an http or https page without cookies, converts HTML to Markdown and returns up to 16,000 characters at a time. Addresses on loopback, private or link-local networks are refused | Asks |
| **Read skill** (`read_skill`) | Reads an enabled skill's instructions or one of its files | Runs without asking |
| MCP tools (`mcp__<server>__<tool>`) | Tools from the MCP servers the user added (shown as server · tool) | Ask, unless set to Always allow |

Results longer than 16,000 characters keep the first 4,000 and the last 12,000. Every call and result is a collapsible block in the conversation, in both modes. Screen text, command output and web pages are data: the assistant is told never to follow instructions found in them.

**Permission mode** (the shield or lightning button in the header; the menu is titled "Permission mode for this conversation"):

| en | zh-CN | ja |
|---|---|---|
| Permission mode | 权限模式 | 権限モード |
| Manual approval | 手动批准 | 手動承認 |
| Bypass | 绕过批准 | 承認をバイパス |
| Bypass | 绕过 | バイパス |
| Turn On Bypass | 绕过批准 | 承認をバイパス |

- **Manual approval** (the default): tools marked "Asks" wait for you.
- **Bypass**: every tool runs without asking, except tools of MCP servers set to **Always ask**. It is riskier because text on the screen or in a page can steer the assistant. Switching to Bypass for the first time on a device shows a confirmation ("Let the assistant act without asking?", button **Turn On Bypass**).
- The switch changes the current conversation on this device until the app quits. The default for new conversations is Settings → AI → **This Device** → **Default Permission Mode** (device-local, not synced).

| en | zh-CN | ja |
|---|---|---|
| AI | AI | AI |
| This Device | 此设备 | このデバイス |
| Default Permission Mode | 默认权限模式 | 既定の権限モード |
| Default Model | 默认模型 | 既定のモデル |
| Always ask | 始终询问 | 常に確認 |

An approval card appears in the conversation for a call that needs approval. It shows the tool, the host and the full input (the command and timeout, the text and key, the URL, or an MCP server with the tool's description and arguments).

| en | zh-CN | ja |
|---|---|---|
| Needs approval | 等待批准 | 承認待ち |
| Run | 运行 | 実行 |
| Edit | 编辑 | 編集 |
| Reject | 拒绝 | 拒否 |
| Allow for This Conversation | 在此对话中允许 | この会話では許可 |
| Always Allow | 总是允许 | 常に許可 |
| Reason (optional, sent to the model) | 理由（可选，会告诉模型） | 理由（任意、モデルに伝えられます） |

- **Run** runs the call. **Edit** changes the input first (the model is told what you changed). **Reject** takes an optional reason that goes back to the model.
- **Allow for This Conversation**: later calls of the same tool in this conversation run without asking until the app quits.
- **Always Allow** (MCP tools only): stored on this device; change it in Settings → AI.
- After the **Tool Call Limit** (25 by default) a turn pauses with "The assistant paused after N tool calls. Let it continue?" and the buttons **Stop** and **Continue**. **Stop**, or Esc while focus is in the panel, ends a turn; it sends nothing to the terminal.

| en | zh-CN | ja |
|---|---|---|
| Continue | 继续 | 続ける |
| Tool Call Limit | 工具调用上限 | ツール呼び出しの上限 |

## Selecting terminal text

Text selected in the tab's terminal appears as a chip above the input ("Selection · N lines") while the panel shows that tab's conversation. It follows the selection as it changes and goes with the next message. Hover or click the chip to preview the first lines, and press × (**Don’t Attach the Selection**) to leave it out. Very long selections are cut like a long tool result and marked "truncated". After a message takes the selection, the chip comes back only when the selection changes. A sent message shows it as a collapsed block above what you typed. The home tab has no selection.

| en | zh-CN | ja |
|---|---|---|
| Preview the selection | 预览选中内容 | 選択範囲をプレビュー |
| Don’t Attach the Selection | 不附带选中内容 | 選択範囲を添付しない |
| truncated | 已截断 | 切り詰め |

## Asking about a failed connection

When a connection fails, the card over the terminal has **Copy Diagnostics**, **Ask AI**, **Edit Host** and **Retry**. **Ask AI** opens the AI panel on that tab and does three things:

- It attaches the connection diagnostics as a chip above the input, titled "Connection diagnostics · <host>". The diagnostics hold the host's name, address and port, user name, authentication kind, jump host, error kind and detail, the number of attempts and the time. They never hold a password, key or passphrase.
- It puts a suggested question in the input, selected, so typing replaces it. This happens only when the input was empty; a draft you already wrote stays.
- It sends nothing. The diagnostics reach the provider only when you press Send. Hover or click the chip to preview exactly what will be sent; × (**Don’t Attach the Diagnostics**) leaves it out.

| en | zh-CN | ja |
|---|---|---|
| Copy Diagnostics | 复制诊断信息 | 診断情報をコピー |
| Edit Host | 编辑主机 | ホストを編集 |
| Retry | 重试 | 再試行 |
| Preview the diagnostics that will be sent | 预览将要发送的诊断信息 | 送信する診断情報をプレビュー |
| Sent with your next message: | 随下一条消息发送： | 次のメッセージと一緒に送信します： |
| Don’t Attach the Diagnostics | 不附带连接诊断 | 接続の診断を添付しない |
| I can’t connect to this host. Can you help me find out why? | 这台主机连不上，帮我看看原因。 | このホストに接続できません。原因を調べてください。 |

A message carries at most one selection and one diagnostics attachment, the diagnostics first. A sent message shows each as a collapsed block above what you typed, and **Edit and Resend** keeps them unless their chip is removed. The diagnostics stay on the tab until they are sent or removed. If no provider is set up yet, the panel first asks for one (Settings → AI). While the tab is not connected the terminal tools are unavailable, so the assistant can explain the error and what to check in Hatoba, but it cannot run anything on the host; `references/troubleshooting-connections.md` explains each error kind.

## Model, context and compaction

- **Model** lists the models of all providers, grouped by provider. A new conversation starts with the default model (Settings → AI → **Default Model**); a conversation keeps the model it used last. Switching keeps the whole conversation.
- The context meter (a ring with a percentage, or a token count when the model's context window is unknown) shows how much of the window the conversation uses; hover for the numbers. "≈" marks an estimate. At 80% it turns to a warning color and offers **Compact**.
- **Compact** (also **Compact Conversation** in the meter's menu) asks the model to summarize the conversation. The summary becomes the start of the context; earlier messages stay visible, marked "Everything above is outside the context". Hatoba also compacts by itself before a request that would pass 90% of the context window.
- Model reasoning, when the provider returns it, is shown above the answer as **Reasoning**, collapsed.

| en | zh-CN | ja |
|---|---|---|
| Context usage | 上下文用量 | コンテキストの使用量 |
| Compact | 压缩 | 圧縮 |
| Compact Conversation | 压缩对话 | 会話を圧縮 |
| Reasoning | 思考过程 | 思考過程 |
| Everything above is outside the context | 以上内容不在上下文中 | ここより上はコンテキストに含まれません |

## History, search, export, edit

**History** lists conversations, pinned first and then by last activity, each with its title, host and time. The title starts as the first line of the first message. Conversations are encrypted and sync with the vault. The search box searches titles and message text. The menu of a conversation (More) has:

| en | zh-CN | ja |
|---|---|---|
| Search conversations | 搜索对话 | 会話を検索 |
| Open | 打开 | 開く |
| Rename | 重命名 | 名前を変更 |
| Pin | 置顶 | ピン留め |
| Unpin | 取消置顶 | ピン留めを外す |
| Export as Markdown… | 导出为 Markdown… | Markdown として書き出す… |
| Delete… | 删除… | 削除… |
| More | 更多操作 | その他の操作 |

- **Delete…** removes the conversation on every synced device for good, after a confirmation.
- **Export as Markdown…** saves the conversation as a Markdown file.
- **Edit and Resend** (a pencil next to one of your messages) edits the text; **Resend** sends it again. It deletes every message after it on all synced devices, after a confirmation (**Delete and Resend**).

| en | zh-CN | ja |
|---|---|---|
| Edit and Resend | 编辑并重新发送 | 編集して再送信 |
| Resend | 重新发送 | 再送信 |
| Delete and Resend | 删除并重新发送 | 削除して再送信 |

## MCP tools menu

The plug button in the input area (**MCP tools**) lists the MCP servers that are on for this device, with their state, and a switch for each. Switching one off removes its tools from this conversation until the app quits. **Manage in Settings → AI** opens the settings. Adding, editing and enabling servers is in `references/ai-settings.md`.

| en | zh-CN | ja |
|---|---|---|
| MCP servers for this conversation | 此对话使用的 MCP 服务器 | この会話で使う MCP サーバー |
| Manage in Settings → AI | 在设置 → AI 中管理 | 設定 → AI で管理 |
| Off for this conversation | 此对话中已关闭 | この会話ではオフ |
| Starts when needed | 需要时启动 | 必要なときに起動 |
| Starting… | 正在启动… | 起動しています… |
| Failed | 出错 | エラー |

Provider and tool errors in the panel are explained in `references/troubleshooting-ai.md`.
