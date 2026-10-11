# AI conversation history, edit and fork

Conversations with the assistant are kept in **History**. They are encrypted and sync with the vault. Using the panel is covered in `references/ai-panel.md`.

## History, search, export

**History** (the clock button in the panel header) lists conversations, pinned first and then by last activity, each with its title, host and time. The title starts as the first line of the first message. The search box searches titles and message text. The menu of a conversation (More, or right-click the row) has:

| en | zh-CN | ja |
|---|---|---|
| Search conversations | 搜索对话 | 会話を検索 |
| Open | 打开 | 開く |
| Fork | 创建分支 | 分岐 |
| Rename | 重命名 | 名前を変更 |
| Pin | 置顶 | ピン留め |
| Unpin | 取消置顶 | ピン留めを外す |
| Export as Markdown… | 导出为 Markdown… | Markdown として書き出す… |
| Delete… | 删除… | 削除… |
| More | 更多操作 | その他の操作 |

- **Open** attaches the conversation to the active tab. On a tab of another host, the next message moves the whole conversation there (`references/ai-panel.md`). To keep it where it is, use **Fork** instead.
- **Delete…** removes the conversation on every synced device for good, after a confirmation.
- **Export as Markdown…** saves the conversation as a Markdown file, with each divider as a line.

## Edit and Resend

**Edit and Resend** (a pencil next to one of your messages) edits the text; **Resend** sends it again. It deletes every message after it on all synced devices, without asking, unless those messages changed files: then the question in `references/ai-file-changes.md` comes first. A divider about a move to another host stays with the message; the other dividers are worked out again for the tab and model it is resent with. To keep the messages after it, fork from the message instead.

| en | zh-CN | ja |
|---|---|---|
| Edit and Resend | 编辑并重新发送 | 編集して再送信 |
| Resend | 重新发送 | 再送信 |

## Fork a conversation

A fork copies a conversation up to a point into a new, separate conversation, which opens in the active tab. The original stays as it was, in History. When the active tab's conversation is working, waiting for approval, compacting or reverting a file edit, the fork does not replace it: it stays in History, and the panel says so. Use it to try another direction from an earlier message, or to go on with a conversation on another host without moving the original there.

- **Fork from Here** (the fork icon) is next to each of your messages, beside Copy, and under each finished reply, beside Copy and the time. A fork from your message includes it: send it again with **Edit and Resend**, or type a new message. A fork from a reply includes the reply with its tool calls and their results.
- **Fork** in a conversation's History menu copies the whole conversation.
- Neither is offered while that conversation is working, waiting for approval, compacting or reverting a file edit, or while a fork of it is being made.
- The fork's title is the original's followed by "(fork)", in the interface language, or "(fork 2)", "(fork 3)" and so on when History has that title already; a fork of a fork drops the earlier mark first. It keeps the original's model and thinking level, and the host the copied messages came from: a fork from before a home tab chat went on in a terminal tab has no host. On a tab of another host, the panel says so, and the fork's next message moves it there with a divider.
- The fork starts like a new conversation: what **Allow for This Conversation** allowed asks again (saved approvals still apply), the permission mode is the default of this device, every MCP server is on, and the assistant has to read a file again before it can edit or replace it. **Revert** on the file edits it copied stays in the original (`references/ai-file-changes.md`).
- After that, the two conversations are independent: nothing added to one appears in the other, on any device. A conversation has no branches to switch between; each fork is a conversation of its own in History.

| en | zh-CN | ja |
|---|---|---|
| Fork from Here | 从这里创建分支 | ここから分岐 |
| Forked | 已创建分支 | 分岐しました |
| Forked. The new conversation is in History. | 已创建分支，新对话在历史对话中。 | 分岐しました。新しい会話は履歴にあります。 |
| Allow for This Conversation | 在此对话中允许 | この会話では許可 |
