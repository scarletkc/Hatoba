# AI assistant panel

The AI assistant is a chat panel at the right edge of the window, below the tab bar. It can read the terminal, run commands, type into the shell, search the web, fetch pages, read skills and use MCP tools, within the permission mode of the conversation. It calls a model provider that the user adds in Settings → AI (`references/ai-settings.md`).

## Opening the panel

- Click the sparkle button at the right of the tab bar, or press Ctrl+Shift+A (⌘⇧A on macOS). The close button (×) in the panel header hides it again. The open state and the width are remembered on this device.
- **Ask AI** in a terminal's right-click menu or **More actions** menu opens the panel on that tab with the input focused; the selected text shows as a chip above the input. It is disabled while nothing is selected, and the right-click menu appears only when **Right-Click in Terminal** is set to **Show context menu** (`references/terminal.md`). **Ask AI** on a failed connection's card attaches its diagnostics.
- Drag the panel's left edge to resize it (300 to 900 px); double-click the edge to reset the width.
- With no provider, the panel says "Add a model provider" with a button that opens Settings → AI. With a provider that has no model, it asks to add a model.

Other labels: **AI Assistant**, **Show AI Panel**, **Hide AI Panel**, **Close AI Panel**, **Open Settings → AI**.

## One conversation per tab

The panel shows the conversation of the active tab, and switching tabs switches the panel. Every terminal tab has its own conversation, and the home tab (host list, Keys, Cloud Sync) has a chat-only one. A tab shows a small sparkle while its assistant is working and a hand while it waits for your approval, also for a conversation still working in the tab's background (below).

- Header: the conversation title, the tab's host, **History**, **New Conversation**, and close.
- Input area: attachment chips, a text box (Enter sends, Shift+Enter adds a line; nothing is sent while an input method is composing; `/` starts a command and `@` adds context, below), under it on the left the + button (**Add Files and More**, below) and the permission mode button, and on the right the context meter, the **Model** picker, **Ask a Side Question**, and **Send (Enter)** (replaced by **Stop** while a turn runs). A line above the box names an MCP server this conversation uses that failed to start, with `/mcp`.
- The + menu: **Add Text Files…**, **Slash Commands** (puts `/` at the start of the box, which opens the command menu), **Send Terminal Screen**, then **Skills** and **MCP Tools**. Those two open a list in the menu's place; its first row, or Left, goes back. Picking a skill puts its command (`/name`) at the start of the box, before what you typed; **MCP Tools** is the list `/mcp` shows (below). The + button and **MCP Tools** show a red dot while a server this conversation uses has failed.
- Sending while the assistant works: Enter queues the message instead of interrupting. It shows above the input box, "Queued: sends when the turn ends", with its text, its attachments, **Edit**, an arrow button and ×, and goes by itself once the turn completes. Enter again adds to it, so the two go as one message. **Edit**, or Up in the empty box, puts it back into the box, before anything typed since; × deletes it. Ctrl+Enter (⌘↩ on macOS) stops the turn and sends at once: the queued message first, then the box, or just the queued message when the box is empty. **Stop**, Esc or the card's arrow button also stops the turn and then sends the queued message (their tooltip says so); to stop without sending it, delete it first. A turn waiting for approval keeps waiting: queuing never approves or rejects a call. When the turn ends any other way (a provider error, a reply cut off, declined or without an answer) or the vault locks, the queued message goes back into the input box unsent; closing the tab discards it. When another conversation opens in the tab, the queued message stays with its conversation and goes when that turn ends in the background (below). Its terminal selection and diagnostics come back as chips too: if the tab has a newer selection or newer diagnostics by then, the returned chip goes with the next message and the newer one waits for a later message. It is kept only in memory, never saved or synced. A message takes at most one terminal selection, so while the queued message has one, a new selection stays in the input area for a later message.
- Messages: each of yours shows the time you sent it, and each reply, once it is finished, the time it ended (hover for the full date and time). **Copy** next to one of your messages copies what you typed, without the attachments; **Copy** under a reply copies its Markdown, without the reasoning and tool calls, and a reply that used tools is copied whole, with its text from before and after the calls. **Fork from Here** beside them copies the conversation up to that point into a new one (`references/ai-history.md`).
- Leaving a conversation does not stop it. **New Conversation**, opening another conversation from History, or a fork opening, while the assistant works or waits for approval keeps that turn going in the tab's background: its commands still run on this tab, a waiting approval card keeps waiting, and a queued message goes when the turn ends. The tab bar keeps its sparkle or hand, and History marks the conversation the same way; open it from History to see where it is, approve the waiting call, or stop it. Only its side question stops. Closing the tab or locking the vault stops it.
- A tab has one shell, so only one conversation's turn at a time types into it: the first turn that types there keeps it until that turn ends. If another conversation of the tab tries to type meanwhile, its call fails at once with a message naming the conversation using the shell, without asking you; the assistant can still run commands with **Run command** (its own channel) and use the file tools, or wait. The tab's conversations also share its SSH connection: at most four of their commands, file tools and host searches run at once, and one web search, and a call past that shows "Queued" until another ends.
- A conversation acts only on its own tab, never on another, in any permission mode. While the tab is disconnected or still connecting, the terminal tools are unavailable (the panel says so and offers **Reconnect**). Closing a tab detaches its conversation; it stays in History.
- Opening a conversation from History attaches it to the active tab. If that tab's host differs, the panel says so, and the next message moves the conversation to the tab's host. That message gets a divider above it, "Moved to <new host> (was on <old host>)", and the assistant is told that the screens and command output before it came from the old host. A conversation that is running a turn, waiting for approval or compacting in another tab, shown there or in its background, is not moved: that tab is brought to the front and shows it, because its turn acts on that tab's terminal. To continue on another host and leave the original where it is, fork it instead.
- A quick connection counts as a host named by its `user@host:port`, in the panel and in History, so a conversation that moves from one to a saved host or to another quick connection gets the same notice and divider. Saving a quick connection as a host (**Save as Host…**) keeps the same server, so a conversation that continues on the saved host gets neither.
- With no terminal tab (the home tab), or with a tab that is not connected, the assistant can still chat, search the web (when it has a web search tool), fetch pages, read skills and use MCP tools. It cannot read the terminal, run commands, send input, or read, edit or write files: those tools are not offered, and a call to one anyway returns an error without asking. For a conversation that belongs to a host or a quick connection, the panel offers a "Connect to <host>" button that opens a tab and attaches the conversation; a quick connection saved as a host since opens as that host.
- A new terminal tab starts with its own empty conversation, and the home tab's chat stays on the home tab. While the tab's conversation has no messages and the home tab's chat has some and is not working, waiting for approval or compacting, the panel offers **Continue Here**, which moves that chat into the tab and gives the home tab a new conversation. Whatever the tab's input holds stays, a selection or diagnostics chip included; when it holds nothing, the home tab's unsent text and attachments come along. The next message moves the chat to the tab's host.
- When the terminal tools come or go between two messages, as when a home tab chat continues in a connected tab or a tab disconnects and connects again, the next message gets a divider above it, "Terminal now available" or "Terminal no longer available", and the assistant is told that its earlier replies had the tools of their time.

Other labels: **Stop and Send the Queued Message (Esc)**, **Remove the Queued Message**, **The assistant is working**, **The assistant is waiting for you**.

## `/` commands and `@` mentions

Type `/` at the start of the box for commands, or `@` at the start of a word to add context. A menu opens above the box and filters as you type; Up and Down move, Enter or Tab picks, Esc closes it. The Enter that commits an input method's candidate never picks.

- `/btw <question>`: a side question (`references/ai-side-questions.md`).
- `/clear` (also `/new`, `/reset`): a new conversation.
- `/compact [instructions]`: compacts the conversation; text after it tells the summary what to keep (`references/ai-models.md`).
- `/mcp`: the MCP servers of this conversation (below).
- `/memory`: opens Settings → AI at **Memory** (`references/ai-instructions.md`).
- `/model`: opens the **Model** picker.
- `/<skill>`: runs an enabled skill, the built-in `/hatoba` included. Its `SKILL.md` goes with the message, which the panel shows as a chip, so the assistant follows it without reading it first.
- `/mcp__<server>__<prompt> [arguments]`: runs a prompt of an MCP server. Words after it fill the prompt's arguments in order, a word in quotes may hold spaces, and the last argument takes the rest. The prompt's text goes with the message.
- Enter on a command in the menu runs it when it needs nothing more; otherwise, and with Tab, it completes the name. A message that starts with `/` and names no command goes as typed.
- `@` lists the home directory of the tab's host; type a path (`@/etc/ng`, `@~/app/`) and pick a folder to go into it. Picking a file attaches it, up to 256 KB of UTF-8 text, and picking the first row of a folder attaches its list of entries. The text keeps `@` and the path.
- `@` also lists the resources of running MCP servers (`@server:uri`), which attach the same way, and **Text Files from This Computer…**, which opens the file picker.

Other labels: **Add context**.

## Tools and approvals

The tools (read the terminal, run commands, type into the shell, read, edit and write files, search the web, fetch pages, read skills, MCP tools), the permission modes (**Manual approval**, **Auto**, **Bypass**) and the approval card (**Run**, **Edit**, **Reject**, **Allow for This Conversation**, **Always Allow**) are in `references/ai-tools.md`.

## Attachments

What goes with a message is covered in `references/ai-attachments.md`: the terminal's screen, the terminal selection, **Ask AI** about a failed connection (diagnostics), long pastes (Ctrl+Shift+V pastes as text), and text files attached from the + menu, the clipboard or a drop on the panel.

## Model, thinking level, context and compaction

The **Model** picker (with its **Thinking Level** row), the context meter, **Compact**, automatic compaction and the reasoning display are in `references/ai-models.md`. Provider and model setup is in `references/ai-settings.md`.

## History, edit and fork

**History** (search, open, rename, pin, export, delete), **Edit and Resend**, and forking a conversation (**Fork from Here**, **Fork**) are in `references/ai-history.md`.

## MCP servers: **MCP Tools** and `/mcp`

**MCP Tools** in the + menu, and `/mcp`, show the MCP servers that are on for this device, with their state, the error and the last lines of stderr of one that failed, and a switch for each. While a server this conversation uses has failed, a red line above the box names it, and its `/mcp` link opens the list; the + button and **MCP Tools** show a red dot. Switching one off removes its tools, prompts and resources from this conversation until the app quits, and stops its calls in a running turn too: they return an error result. **Manage in Settings → AI** opens the settings. Adding, editing and enabling servers is in `references/ai-mcp.md`. Opening the `/` or `@` menu starts the servers this conversation uses, as its next message would, so that their prompts and resources are listed.

Once a conversation's MCP tool definitions would take a tenth of the model's context window (3,200 tokens when it is not set), or a request would carry over 128 tools in all, they stop going with each request from then on, which keeps the provider's prompt cache working. A message lists the tools by name instead, shown as a divider "Listed N MCP tools" (列出了 N 个 MCP 工具, MCP ツールを N 個一覧にしました). The assistant calls a listed tool directly, or finds one with **Search MCP tools**.

Other labels: **MCP servers for this conversation**, **Off for this conversation**, **Starts when needed**, **Starting…**, **Failed**, **Starting MCP servers…**.

Provider and tool errors in the panel are explained in `references/troubleshooting-ai.md`.
