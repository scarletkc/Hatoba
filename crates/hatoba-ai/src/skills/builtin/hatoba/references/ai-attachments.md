# Attachments to an AI message

A message to the assistant can carry attachments that go with it when the user sends it. Nothing is sent when an attachment is added; it reaches the model provider only when the user sends the message. Attachments are sent as part of the message, so they are stored in the conversation, sync with it, and appear in an exported conversation. Using the panel is covered in `references/ai-panel.md`.

| Kind | Where it comes from | Chip label |
|---|---|---|
| Connection diagnostics | **Ask AI** on a failed connection's card | Connection diagnostics · host |
| Terminal selection | Text selected in the tab's terminal | Selection · N lines |
| Pasted text | A long paste into the input | Pasted text · N lines |
| Text file | **Add Text Files…** in the + menu, the clipboard, or dropping on the panel | file name · N lines |
| File on the host | A file picked after `@` | file name · N lines |
| Folder on the host | The first row of a folder after `@`: its list of entries | name/ · N entries |
| MCP resource | A resource picked after `@` | name · N lines |
| Skill | `/<skill>` at the start of the message | Skill /name |
| MCP prompt | `/mcp__<server>__<prompt>` at the start of the message | Prompt name |

A message carries at most one diagnostics attachment and one selection, and any number of the others. They are stored in this order: diagnostics, selection, then the others in the order they were added, a skill or prompt that the message's `/` command attached first. `@` and `/` are covered in `references/ai-panel.md`. A file or folder on the host shows the assistant its content as it was when you sent the message, with its path; to change the file, the assistant still reads it with **Read file** first. The home tab has no selection or diagnostics, but it can take pastes and files. Pending attachments stay in the tab's input until they are sent or removed, even if another conversation is opened in the same tab.

## Terminal selection

Text selected in the tab's terminal appears as a chip above the input while the panel shows that tab's conversation. It follows the selection as it changes. **Ask AI** in the terminal's menus (`references/terminal.md`) opens the panel with the input focused. Hover or click the chip to preview the first lines, and press × to leave it out. Very long selections are cut like a long tool result (first 7,500 and last 22,500 characters) and marked "truncated". After a message takes the selection, the chip comes back only when the selection changes.

Other labels: **Preview the selection**, **Don’t Attach the Selection**.

## Asking about a failed connection

When a connection fails, the card over the terminal has **Copy Diagnostics**, **Ask AI**, **Edit Host** and **Retry**. **Ask AI** opens the AI panel on that tab and does three things:

- It attaches the connection diagnostics as a chip titled "Connection diagnostics · <host>". They hold the host's name, address and port, user name, authentication kind, jump host, proxy, error kind and detail, the number of attempts and the time, and never a password, key or passphrase. Like a selection, a very long text is cut.
- It puts a suggested question in the input, selected, so typing replaces it. This happens only when the input was empty; a draft you already wrote stays.
- It sends nothing. Hover or click the chip to preview exactly what will be sent; × (**Don’t Attach the Diagnostics**) leaves it out. They stay on the tab until they are sent or removed.

Other labels: **Preview the diagnostics that will be sent**.

Other messages:

- "Sent with your next message:"
- "I can’t connect to this host. Can you help me find out why?"

While the tab is not connected the terminal tools are unavailable, so the assistant can explain the error and what to check in Hatoba but cannot run anything on the host; `references/troubleshooting-connections.md` explains each error kind. If no provider is set up yet, the panel first asks for one (Settings → AI).

## Long pastes

A paste of at least 2,000 characters or 30 lines into the input box (or into the edit box of **Edit and Resend**) does not go into the box. It becomes a chip, "Pasted text · N lines", with an estimate of its tokens (≈N tokens). The text is kept whole, never cut.

- Hover or click the chip to preview the text. The preview has **Paste as Text**, which removes the chip and puts the text into the box at the caret. Its tooltip says that Ctrl+Shift+V (⌘⇧V on macOS) always pastes as text.
- Ctrl+Shift+V (⌘⇧V) in the input box pastes the clipboard as plain text however long it is, without making a chip. A shorter paste with Ctrl+V goes into the box as usual.
- × (**Remove the Pasted Text**) removes the chip.

Other labels: **Preview the pasted text**.

## Text files

**Add Text Files…** in the input area's + menu (the + button, **Add Files and More**) opens a file picker for one or more files. Files can also be attached by copying them in a file manager and pasting into the input box, or by dropping them on the panel (a "Drop text files to attach them" overlay shows while dragging; it works in the desktop app too, which reads the dropped paths itself). Dropping needs a provider with a model to be set up. Each file becomes a chip "name · N lines" with a preview and ×.

- Only UTF-8 text files are accepted: source code, configs, logs, Markdown, JSON. A file with NUL bytes or invalid UTF-8 is refused as not a text file. Images (PNG, JPEG, GIF, WebP, SVG and so on) are not supported yet. Documents such as PDF or Word files are binary and are refused too.
- A file over 256 KB is refused. All attachments of one message together (diagnostics, selection, pastes and files) may not pass 512 KB; the one that would pass it is refused. Each refusal shows a message naming the file, for example "<name> is larger than 256 KB, so it can’t be attached." or "A message’s attachments can’t exceed 512 KB in total."
- Only the file's base name is sent, never its folder path, which could reveal a local user name. The text is kept whole, never cut.

Other labels: **Preview the file**.

## The terminal's screen

Each message sent while a connected terminal tab is attached also shows the assistant that tab's screen as it is at that moment: the last 20 lines, or the whole screen while a full-screen program such as vim, less or htop runs, never the scrollback. Hatoba adds it when it stores the message, so no chip shows in the input. The sent message shows it as a collapsed block, "Terminal screen · N lines" with the host, marked "full-screen program" when one was running. Like attachments, it is stored and synced with the conversation and appears in an export, but History search skips it. The assistant reads the screen again only when it needs earlier lines or the screen may have changed since. To stop sending it in a conversation, turn off **Send Terminal Screen** in the input area's + menu; that holds for the conversation on this device until the app quits. Without a connected terminal the switch is off and says "Needs a connected terminal". A conversation where it was not changed follows **Send with Each Message by Default** under Settings → AI → **Terminal Screen** (synced, on by default).

## Does it fit the model?

The input area counts the message with its attachments (about 4 characters per token). When the message would bring the conversation past 80% of the model's context window, a note says "With this message the conversation uses about N% of the model’s context. Past 90%, it is compacted before sending." Past 90%, Hatoba compacts the earlier conversation before the request. A message larger than the whole window cannot be sent, because compacting cannot shrink it: the note says "This message is about N tokens, more than the model’s context window… Remove attachments or choose a model with a larger context.", and Send is disabled. The check needs the model's **Context** window to be set (`references/ai-settings.md`).

Other labels: **The message is larger than the model’s context window**.

## After sending

A sent message shows each attachment as a collapsed card above what you typed; click a card to open its text. **Export as Markdown…** writes each attachment as a labelled fenced block. **Edit and Resend** keeps the attachments with the message unless their chip is removed, and a long paste in its edit box becomes another attachment. If a message is stopped before it was stored (for example during the compaction before it), its text and its pastes and files return to the input.
