# AI tools, permission modes and approvals

What the assistant can call, which calls wait for the user, and the approval card. The panel itself is covered in `references/ai-panel.md`.

## Tools

| Tool | What it does | Manual approval mode |
|---|---|---|
| **Read terminal** (`read_terminal`) | Returns the terminal's last lines across the screen and the scrollback as text (20 by default, or a full-screen program's whole screen; at most 1,000), with the total line count and whether a full-screen program (vim, htop) is active | Runs without asking |
| **Run command** (`run_command`) | Runs a command on a new exec channel of the tab's SSH connection and returns stdout, stderr and the exit status. It has no PTY, does not share the shell's directory, environment or sudo session, and its output does not show in the terminal. It starts in the home directory, or in a folder the assistant names, absolute or from the home directory; when that folder can't be entered, nothing runs. Timeout 120 s by default, at most 600 s. A stop or the timeout ends the command and the processes it started, and the result says whether they ended | Asks, unless it only reads (`ls`, `git status`, …) or is allowed |
| **Type in terminal** (`send_input`) | Types text into the shell as the keyboard would, then presses keys in order: Enter, Tab, Esc, the arrows and other navigation keys, F1–F12, Ctrl and Alt combinations and single characters (Ctrl+X then N leaves nano without saving). Then it waits for output (10 s by default, at most 120 s). A command started this way keeps running if the turn is stopped. While another conversation of the tab is using the shell, it fails at once and names that conversation | Asks, unless allowed |
| **Read file** (`read_file`) | Returns a text file on the tab's host as numbered lines, a page of up to 30,000 characters at a time. Binary files and anything but regular files are refused; edits and writes refuse files over 1 MB. Like the other file tools it never uses sudo | Runs without asking |
| **Edit file** (`edit_file`) | Replaces an exact piece of text in a file on the host and writes the file back in place, keeping its owner, mode and line breaks. Refused, in every mode, unless the assistant read the file in this conversation and it has not changed since. The approval card shows the diff; **Edit** changes the new text | Asks, unless allowed |
| **Write file** (`write_file`) | Creates a file on the host or replaces its content, which needs the same read as Edit file. The approval card shows the content, or a diff for an existing file. Both can be reverted (`references/ai-file-changes.md`) | Asks, unless allowed |
| **Search in files** (`grep`) | Searches the text files of a file or directory on the host for lines that match a regular expression and returns the files, the number of matching lines in each, or the lines with some around them, sorted by path. Binary files and `.git` directories are skipped; like the file tools it never uses sudo | Runs without asking |
| **Find files** (`glob`) | Lists the files on the host whose paths match a pattern such as `**/*.conf`, sorted by path, at most 1,000 | Runs without asking |
| **Web search** (`web_search`) | Searches with the search provider chosen in Settings → AI. With Native Search (the default), the current model searches with its provider's own search and returns the pages it found with its account of them. Not offered with None | Runs without asking |
| **Fetch page** (`fetch_url`) | Fetches an http or https page without cookies, converts HTML to Markdown and returns up to 30,000 characters at a time. Of an HTML page it returns the main content, without navigation, sidebars and footers, unless the page does not read as one text or the assistant asks for the whole page. Addresses on loopback, private or link-local networks are refused | Asks, unless allowed |
| **Read skill** (`read_skill`) | Reads an enabled skill's instructions or one of its files | Runs without asking |
| **Memory** (`memory`) | Saves a memory under a name, replacing the one of that name, or deletes one (`references/ai-instructions.md`). Offered while memory is on | Runs without asking |
| MCP tools (`mcp__<server>__<tool>`) | Tools from the MCP servers the user added (shown as server · tool) | Ask, unless set to Always allow |
| **Search MCP tools** (`search_tools`) | With many MCP tools (see `/mcp` in `references/ai-panel.md`), finds them by keywords or name and returns their descriptions and input schemas | Runs without asking |
| `call_mcp_tool` | Calls an MCP tool by name; shown, approved and allowed as that tool | As that tool |

Results longer than 30,000 characters keep the first 7,500 and the last 22,500 (MCP tools: 50,000). Every call and result is a collapsible block in the conversation, in every mode. Calls of one reply that only read and don't wait for approval run at the same time; other calls run one at a time, in order. Tool results are data to the assistant: when one asks for something, it tells you instead.

## On a local terminal tab

In a local terminal tab (a shell of this computer, see `references/terminal.md`), the assistant has the same tools, permission modes and approvals as in an SSH tab, and they act on this computer. Approval cards and the panel say **This Computer** and the shell, and **Always allow** there covers this computer's local tabs. In a WSL distribution's tab, commands and the file and search tools act inside the distribution, on its Linux paths. In PowerShell, Command Prompt and Git Bash they act on Windows, with Windows paths (`C:/Users/name/file.txt`; `~` is the home folder), and `run_command` runs the command in that shell without its profile; Command Prompt takes one line. A stop or a timeout ends the command and everything it started. Moving a conversation between a host and a local tab is noted like a move between hosts.

## Permission mode

The shield, robot or lightning button next to the + button in the input area (the robot also shows "Auto" and the lightning "Bypass" unless the panel is narrow); the menu is titled "Permission mode for this conversation".

- **Manual approval** (the default): tools marked "Asks" wait for you.
- **Auto**: a call that Manual approval would ask about first goes to a judge, a model chosen in Settings → AI → **Auto Mode** (`references/ai-settings.md`). The call runs without the card when the judge finds that it only reads, or that it is a change you asked for that deletes nothing, touches no credentials, sends nothing out, grants no access, sets up nothing that persists, weakens no security and runs no code it can't see in full; any other call shows the card, which says what concerned the judge, with its probabilities, or why it could not decide (no judge, no answer in time, an error). The judge never blocks a call. These always show the card without asking the judge: tools of MCP servers set to **Always ask** and tools that can't be found; input with hidden characters, in MCP arguments too; keys that edit the terminal's line and then submit it in the same call; a command that names recursive `rm`, `shred`, `dd`, `mkfs`, `wipefs`, `fdisk`, `sfdisk`, `parted`, `shutdown`, `reboot`, `poweroff` or `halt` (even as an argument), also in MCP arguments or completing what the terminal's line already holds; typing while the screen asks for a password; and, once you reject a call, the rest of that reply's calls. The reason you give when rejecting reaches the judge for later calls. `sudo` alone does not. While the judge checks a call, its block shows "Checking"; a call Auto ran shows **Auto**, and opening the block shows the judge's answers. Auto needs a judge: until one is chosen, the menu shows it disabled with "Needs a judge" and offers "Choose a Judge in Settings → AI". No confirmation is asked.
- What the judge receives: the text you typed in the conversation (attachments are only named), your custom instructions and the host's AI notes, the host's name, the arguments of the assistant's calls, and for typing into the terminal the screen's last 10 lines. Never command output, files it read or the assistant's replies. A judge on another service than the conversation's model receives them too.
- **Bypass**: every tool runs without asking, except tools of MCP servers set to **Always ask**. It is riskier: text on the screen or in a page can steer the assistant. Switching to Bypass for the first time on a device shows a confirmation ("Let the assistant act without asking?", button **Turn On Bypass**).
- The switch changes the current conversation on this device until the app quits. The default for new conversations is Settings → AI → **This Device** → **Default Permission Mode** (device-local, not synced).
- The assistant is told which mode the conversation is in. In Manual approval and Auto it calls the tool and lets the approval card ask, instead of asking first in chat. In Bypass it asks in chat before it deletes data, does something that cannot be undone, restarts a service or affects other users, unless you asked for exactly that. A switch reaches the assistant with your next message, which gets a divider above it, "Permission mode: Auto", "Permission mode: Bypass" or "Permission mode: Manual approval". A conversation's first message tells the assistant the mode it starts in without a divider. The modes go from Manual approval, the strictest, to Auto and Bypass: a switch to a less strict mode while it is working applies from your next message, and a switch to a stricter one applies at once. Switching does not change the system prompt, so the provider's prompt cache keeps working, and compacting the conversation keeps the mode.

Other labels: **Default Model**.

### When Auto cannot judge

When the judge could not decide, the call's card says why:

- "Auto could not judge this call: no judge is available. Choose one in Settings → AI."
- "Auto could not judge this call: it is too long for the judge."
- "Auto could not judge this call: the judge did not answer in time."
- "Auto could not judge this call: the judge could not be reached."
- "Auto could not judge this call: the judge refused the request."
- "Auto could not judge this call: the judge’s answer could not be read."

The judge has 10 seconds (a System One model) or 30 seconds (a chat model), and one retry after a network error, a rate limit or a server error. A call too long for it is one that writes or sends a lot of text. For the others, **Test** beside the judge (`references/ai-settings.md`) shows whether it answers; a refusal usually means a wrong key or model ID, and an unreadable answer a chat model that does not follow forced tool calls, where a System One model or another chat model helps.

## Approval cards

An approval card appears in the conversation for a call that needs approval. It shows the tool, the host and the full input (the command with its **Folder**, when the assistant named one, and timeout, the text and keys, the URL, or an MCP server with the tool's description and arguments). Nothing in the input is hidden: every line shows, with each line break marked ↵, and a chip gives the number of lines when there are several. Hidden control and bidirectional characters (a tab, an escape sequence, a text-direction override) show as escapes such as `\t`, `\x1b` or `\u{202e}`, with a chip "Hidden characters shown as escapes"; in text to type, control characters show as `⟦^X⟧` or `⟦Esc⟧`.

Other labels: **Needs approval**, **Home folder**, **Always Allow on This Host**, **Reason (optional, sent to the model)**.

- **Run** runs the call. **Edit** changes the input first (the model is told what you changed); for a command, also its folder, where empty means the home folder; for text to type, also the keys pressed after it. **Reject** takes an optional reason that goes back to the model.
- **Allow for This Conversation** lets later calls it covers run without asking until the app quits; **Always Allow** saves that on this device. The buttons name what they cover, such as "Always Allow “git pull” on This Host" or "Always Allow example.com"; **Saved Approvals** in `references/ai-settings.md` has the rules. An edit or write card that shows why it can't apply offers neither, and a call whose tool cannot be named (its server was deleted, say) always asks, in every mode.
- **Stop**, or Esc while focus is in the panel, ends a turn; it sends nothing to the terminal. A command from Run command is ended too, along with the processes it started, and Stop waits a few seconds for that. A reply stopped mid-stream keeps its text so far, and the next message gets a "Previous reply stopped" divider.
