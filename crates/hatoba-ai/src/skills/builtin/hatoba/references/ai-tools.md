# AI tools, permission modes and approvals

What the assistant can call, which calls wait for the user, and the approval card. The panel itself is covered in `references/ai-panel.md`.

## Tools

| en | zh-CN | ja |
|---|---|---|
| Read terminal | 读取终端 | ターミナルを読み取る |
| Run command | 运行命令 | コマンドを実行 |
| Type in terminal | 向终端输入 | ターミナルに入力 |
| Read file | 读取文件 | ファイルを読む |
| Edit file | 编辑文件 | ファイルを編集 |
| Write file | 写入文件 | ファイルに書き込む |
| Web search | 搜索网页 | Web 検索 |
| Fetch page | 读取网页 | ページを取得 |
| Read skill | 读取技能 | スキルを読む |
| Search MCP tools | 搜索 MCP 工具 | MCP ツールを検索 |

| Tool | What it does | Manual approval mode |
|---|---|---|
| **Read terminal** (`read_terminal`) | Returns the terminal's last lines across the screen and the scrollback as text (20 by default, or a full-screen program's whole screen; at most 1,000), with the total line count and whether a full-screen program (vim, htop) is active | Runs without asking |
| **Run command** (`run_command`) | Runs a command on a new exec channel of the tab's SSH connection and returns stdout, stderr and the exit status. It has no PTY, does not share the shell's directory, environment or sudo session, and its output does not show in the terminal. Timeout 120 s by default, at most 600 s | Asks, unless it only reads (`ls`, `git status`, …) or is allowed |
| **Type in terminal** (`send_input`) | Types text into the shell as the keyboard would, optionally followed by Enter, Tab, Esc, Ctrl+C, Ctrl+D or an arrow key, then waits for output (10 s by default, at most 120 s). A command started this way keeps running if the turn is stopped | Asks, unless allowed |
| **Read file** (`read_file`) | Returns a text file on the tab's host as numbered lines, a page of up to 30,000 characters at a time. Binary files and anything but regular files are refused; edits and writes refuse files over 1 MB. Like the other file tools it never uses sudo | Runs without asking |
| **Edit file** (`edit_file`) | Replaces an exact piece of text in a file on the host and writes the file back in place, keeping its owner, mode and line breaks. Refused, in every mode, unless the assistant read the file in this conversation and it has not changed since. The approval card shows the diff; **Edit** changes the new text | Asks, unless allowed |
| **Write file** (`write_file`) | Creates a file on the host or replaces its content, which needs the same read as Edit file. The approval card shows the content, or a diff for an existing file. Both can be reverted (`references/ai-file-changes.md`) | Asks, unless allowed |
| **Web search** (`web_search`) | Searches with the search provider chosen in Settings → AI. Not offered when none is chosen | Runs without asking |
| **Fetch page** (`fetch_url`) | Fetches an http or https page without cookies, converts HTML to Markdown and returns up to 30,000 characters at a time. Addresses on loopback, private or link-local networks are refused | Asks, unless allowed |
| **Read skill** (`read_skill`) | Reads an enabled skill's instructions or one of its files | Runs without asking |
| MCP tools (`mcp__<server>__<tool>`) | Tools from the MCP servers the user added (shown as server · tool) | Ask, unless set to Always allow |
| **Search MCP tools** (`search_tools`) | With many MCP tools (see the MCP tools menu in `references/ai-panel.md`), finds them by keywords or name and returns their descriptions and input schemas | Runs without asking |
| `call_mcp_tool` | Calls an MCP tool by name; shown, approved and allowed as that tool | As that tool |

Results longer than 30,000 characters keep the first 7,500 and the last 22,500 (MCP tools: 50,000). Every call and result is a collapsible block in the conversation, in every mode. Tool results are data to the assistant: when one asks for something, it tells you instead.

## Permission mode

The shield, robot or lightning button next to the + button in the input area (the robot also shows "Auto" and the lightning "Bypass" unless the panel is narrow); the menu is titled "Permission mode for this conversation".

| en | zh-CN | ja |
|---|---|---|
| Permission mode | 权限模式 | 権限モード |
| Manual approval | 手动批准 | 手動承認 |
| Auto | 自动批准 | 自動承認 |
| Auto | 自动 | 自動 |
| Bypass | 绕过批准 | 承認をバイパス |
| Bypass | 绕过 | バイパス |
| Turn On Bypass | 绕过批准 | 承認をバイパス |
| Permission mode: Bypass | 权限模式：绕过批准 | 権限モード：承認をバイパス |
| Permission mode: Auto | 权限模式：自动批准 | 権限モード：自動承認 |
| Permission mode: Manual approval | 权限模式：手动批准 | 権限モード：手動承認 |
| Needs a judge | 需要判定模型 | 判定モデルが必要 |
| Choose a Judge in Settings → AI | 在设置 → AI 中选择判定模型 | 設定 → AI で判定モデルを選ぶ |
| Checking | 判定中 | 判定中 |

- **Manual approval** (the default): tools marked "Asks" wait for you.
- **Auto**: a call that Manual approval would ask about first goes to a judge, a model chosen in Settings → AI → **Auto Mode** (`references/ai-settings.md`). The call runs without the card when the judge finds that it only reads, or that it is a change you asked for that deletes nothing, touches no credentials, sends nothing out, grants no access, sets up nothing that persists, weakens no security and runs no code it can't see in full; any other call shows the card, which says what concerned the judge, with its probabilities, or why it could not decide (no judge, no answer in time, an error). The judge never blocks a call. These always show the card without asking the judge: tools of MCP servers set to **Always ask** and tools that can't be found; input with hidden characters, in MCP arguments too; a command that names recursive `rm`, `shred`, `dd`, `mkfs`, `wipefs`, `fdisk`, `sfdisk`, `parted`, `shutdown`, `reboot`, `poweroff` or `halt` (even as an argument), also in MCP arguments or completing what the terminal's line already holds; typing while the screen asks for a password; and, once you reject a call, the rest of that reply's calls. The reason you give when rejecting reaches the judge for later calls. `sudo` alone does not. While the judge checks a call, its block shows "Checking"; a call Auto ran shows **Auto**, and opening the block shows the judge's answers. Auto needs a judge: until one is chosen, the menu shows it disabled with "Needs a judge" and offers "Choose a Judge in Settings → AI". No confirmation is asked.
- What the judge receives: the text you typed in the conversation (attachments are only named), your custom instructions and the host's AI notes, the host's name, the arguments of the assistant's calls, and for typing into the terminal the screen's last 10 lines. Never command output, files it read or the assistant's replies. A judge on another service than the conversation's model receives them too.
- **Bypass**: every tool runs without asking, except tools of MCP servers set to **Always ask**. It is riskier: text on the screen or in a page can steer the assistant. Switching to Bypass for the first time on a device shows a confirmation ("Let the assistant act without asking?", button **Turn On Bypass**).
- The switch changes the current conversation on this device until the app quits. The default for new conversations is Settings → AI → **This Device** → **Default Permission Mode** (device-local, not synced).
- The assistant is told which mode the conversation is in. In Manual approval and Auto it calls the tool and lets the approval card ask, instead of asking first in chat. In Bypass it asks in chat before it deletes data, does something that cannot be undone, restarts a service or affects other users, unless you asked for exactly that. A switch reaches the assistant with your next message, which gets a divider above it, "Permission mode: Auto", "Permission mode: Bypass" or "Permission mode: Manual approval" (a first message gets one only in Auto or Bypass). The modes go from Manual approval, the strictest, to Auto and Bypass: a switch to a less strict mode while it is working applies from your next message, and a switch to a stricter one applies at once. Switching does not change the system prompt, so the provider's prompt cache keeps working, and compacting the conversation keeps the mode.

| en | zh-CN | ja |
|---|---|---|
| AI | AI | AI |
| This Device | 此设备 | このデバイス |
| Default Permission Mode | 默认权限模式 | 既定の権限モード |
| Default Model | 默认模型 | 既定のモデル |
| Always ask | 始终询问 | 常に確認 |

### When Auto cannot judge

When the judge could not decide, the call's card says why:

| en | zh-CN | ja |
|---|---|---|
| Auto could not judge this call: no judge is available. Choose one in Settings → AI. | 自动批准模式无法判定这次调用：没有可用的判定模型。请在设置 → AI 中选择一个。 | 自動承認モードはこの呼び出しを判定できませんでした：使える判定モデルがありません。設定 → AI で選んでください。 |
| Auto could not judge this call: it is too long for the judge. | 自动批准模式无法判定这次调用：调用的内容太长，判定模型读不下。 | 自動承認モードはこの呼び出しを判定できませんでした：呼び出しの内容が長すぎて判定モデルが読み込めません。 |
| Auto could not judge this call: the judge did not answer in time. | 自动批准模式无法判定这次调用：判定模型没有及时回答。 | 自動承認モードはこの呼び出しを判定できませんでした：判定モデルが時間内に回答しませんでした。 |
| Auto could not judge this call: the judge could not be reached. | 自动批准模式无法判定这次调用：无法连接判定模型。 | 自動承認モードはこの呼び出しを判定できませんでした：判定モデルに接続できませんでした。 |
| Auto could not judge this call: the judge refused the request. | 自动批准模式无法判定这次调用：判定模型拒绝了请求。 | 自動承認モードはこの呼び出しを判定できませんでした：判定モデルがリクエストを拒否しました。 |
| Auto could not judge this call: the judge’s answer could not be read. | 自动批准模式无法判定这次调用：无法读取判定模型的回答。 | 自動承認モードはこの呼び出しを判定できませんでした：判定モデルの回答を読み取れませんでした。 |

The judge has 10 seconds (a System One model) or 30 seconds (a chat model), and one retry after a network error, a rate limit or a server error. A call too long for it is one that writes or sends a lot of text. For the others, **Test** beside the judge (`references/ai-settings.md`) shows whether it answers; a refusal usually means a wrong key or model ID, and an unreadable answer a chat model that does not follow forced tool calls, where a System One model or another chat model helps.

## Approval cards

An approval card appears in the conversation for a call that needs approval. It shows the tool, the host and the full input (the command and timeout, the text and key, the URL, or an MCP server with the tool's description and arguments). Nothing in the input is hidden: every line shows, with each line break marked ↵, and a chip gives the number of lines when there are several. Hidden control and bidirectional characters (a tab, an escape sequence, a text-direction override) show as escapes such as `\t`, `\x1b` or `\u{202e}`, with a chip "Hidden characters shown as escapes".

| en | zh-CN | ja |
|---|---|---|
| Needs approval | 等待批准 | 承認待ち |
| Run | 运行 | 実行 |
| Edit | 编辑 | 編集 |
| Reject | 拒绝 | 拒否 |
| Allow for This Conversation | 在此对话中允许 | この会話では許可 |
| Always Allow | 总是允许 | 常に許可 |
| Always Allow on This Host | 在此主机上总是允许 | このホストでは常に許可 |
| Reason (optional, sent to the model) | 理由（可选，会告诉模型） | 理由（任意、モデルに伝えられます） |
| Hidden characters shown as escapes | 不可见的字符已显示为转义 | 見えない文字をエスケープで表示 |

- **Run** runs the call. **Edit** changes the input first (the model is told what you changed). **Reject** takes an optional reason that goes back to the model.
- **Allow for This Conversation** lets later calls it covers run without asking until the app quits; **Always Allow** saves that on this device. The buttons name what they cover, such as "Always Allow “git pull” on This Host" or "Always Allow example.com"; **Saved Approvals** in `references/ai-settings.md` has the rules. An edit or write card that shows why it can't apply offers neither, and a call whose tool cannot be named (its server was deleted, say) always asks, in every mode.
- **Stop**, or Esc while focus is in the panel, ends a turn; it sends nothing to the terminal. A reply stopped mid-stream keeps its text so far, and the next message gets a "Previous reply stopped" divider.
