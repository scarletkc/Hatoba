# AI conversation history, edit and fork

Conversations with the assistant are kept in **History**. They are encrypted and sync with the vault. Using the panel is covered in `references/ai-panel.md`.

## History, search, export

**History** (the clock button in the panel header) lists conversations, pinned first and then by last activity, each with its title, host and time. The title starts as the first line of the first message. The search box (**Search conversations**) searches titles and message text. The menu of a conversation (**More**, or right-click the row) has **Open**, **Fork**, **Rename**, **Pin** or **Unpin**, **Export as Markdown…** and **Delete…**:

- **Open** attaches the conversation to the active tab. On a tab of another host, the next message moves the whole conversation there (`references/ai-panel.md`). To keep it where it is, use **Fork** instead. A conversation the tab showed that is still working goes on in the tab's background.
- A sparkle beside a title marks a conversation that is working, in a tab or in a tab's background, and a hand one that waits for your approval. Opening one that works in another tab brings that tab to the front and shows it there.
- **Delete…** removes the conversation on every synced device for good, after a confirmation.
- **Export as Markdown…** saves the conversation as a Markdown file, with each divider as a line.

## Edit and Resend

**Edit and Resend** (a pencil next to one of your messages) edits the text; **Resend** sends it again. It deletes every message after it on all synced devices, without asking, unless those messages changed files: then the question in `references/ai-file-changes.md` comes first. A divider about a move to another host stays with the message; the other dividers are worked out again for the tab, model and permission mode it is resent with, and the terminal's screen is read again. To keep the messages after it, fork from the message instead.

## Fork a conversation

A fork copies a conversation up to a point into a new, separate conversation, which opens in the active tab. The original stays as it was, in History. When the active tab's conversation is still working or waiting for approval, it goes on in the tab's background, as when you open another conversation. If you start something else in the tab while the fork is being made (open another conversation, send a message, Compact), or the tab's conversation is reverting a file edit, the fork does not replace it: it stays in History, and the panel says "Forked. The new conversation is in History." Use it to try another direction from an earlier message, or to go on with a conversation on another host without moving the original there.

- **Fork from Here** (the fork icon) is next to each of your messages, beside Copy, and under each finished reply, beside Copy and the time. A fork from your message includes it: send it again with **Edit and Resend**, or type a new message. A fork from a reply includes the reply with its tool calls and their results.
- **Fork** in a conversation's History menu copies the whole conversation.
- Neither is offered while that conversation is working, waiting for approval, compacting or reverting a file edit, or while a fork of it is being made.
- The fork's title is the original's followed by "(fork)", in the interface language, or "(fork 2)", "(fork 3)" and so on when History has that title already; a fork of a fork drops the earlier mark first. It keeps the original's model and thinking level, and the host the copied messages came from: a fork from before a home tab chat went on in a terminal tab has no host. On a tab of another host, the panel says so, and the fork's next message moves it there with a divider. A fork that opens in the tab shows "Forked".
- The fork starts like a new conversation: what **Allow for This Conversation** allowed asks again (saved approvals still apply), the permission mode is the default of this device, every MCP server is on, and the assistant has to read a file again before it can edit or replace it. **Revert** on the file edits it copied stays in the original (`references/ai-file-changes.md`).
- After that, the two conversations are independent: nothing added to one appears in the other, on any device. A conversation has no branches to switch between; each fork is a conversation of its own in History.
