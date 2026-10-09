# Attachments to an AI message

A message to the assistant can carry attachments that go with it when the user sends it. Nothing is sent when an attachment is added; it reaches the model provider only when the user sends the message. Attachments are sent as part of the message, so they are stored in the conversation, sync with it, and appear in an exported conversation. Using the panel is covered in `references/ai-panel.md`.

| Kind | Where it comes from | Chip label |
|---|---|---|
| Connection diagnostics | **Ask AI** on a failed connection's card | Connection diagnostics · host |
| Terminal selection | Text selected in the tab's terminal | Selection · N lines |
| Pasted text | A long paste into the input | Pasted text · N lines |
| Text file | The paperclip, the clipboard, or dropping on the panel | file name · N lines |

A message carries at most one diagnostics attachment and one selection, and any number of pastes and files. They are stored in this order: diagnostics, selection, then pastes and files in the order they were added. The home tab has no selection or diagnostics, but it can take pastes and files. Pending attachments stay in the tab's input until they are sent or removed, even if another conversation is opened in the same tab.

## Terminal selection

Text selected in the tab's terminal appears as a chip above the input while the panel shows that tab's conversation. It follows the selection as it changes. **Ask AI** in the terminal's menus (`references/terminal.md`) opens the panel with the input focused. Hover or click the chip to preview the first lines, and press × to leave it out. Very long selections are cut like a long tool result (first 4,000 and last 12,000 characters) and marked "truncated". After a message takes the selection, the chip comes back only when the selection changes.

| en | zh-CN | ja |
|---|---|---|
| Ask AI | 询问 AI | AI に質問 |
| Preview the selection | 预览选中内容 | 選択範囲をプレビュー |
| Don’t Attach the Selection | 不附带选中内容 | 選択範囲を添付しない |
| truncated | 已截断 | 切り詰め |

## Asking about a failed connection

When a connection fails, the card over the terminal has **Copy Diagnostics**, **Ask AI**, **Edit Host** and **Retry**. **Ask AI** opens the AI panel on that tab and does three things:

- It attaches the connection diagnostics as a chip titled "Connection diagnostics · <host>". They hold the host's name, address and port, user name, authentication kind, jump host, proxy, error kind and detail, the number of attempts and the time, and never a password, key or passphrase. Like a selection, a very long text is cut.
- It puts a suggested question in the input, selected, so typing replaces it. This happens only when the input was empty; a draft you already wrote stays.
- It sends nothing. Hover or click the chip to preview exactly what will be sent; × (**Don’t Attach the Diagnostics**) leaves it out. They stay on the tab until they are sent or removed.

| en | zh-CN | ja |
|---|---|---|
| Copy Diagnostics | 复制诊断信息 | 診断情報をコピー |
| Edit Host | 编辑主机 | ホストを編集 |
| Retry | 重试 | 再試行 |
| Preview the diagnostics that will be sent | 预览将要发送的诊断信息 | 送信する診断情報をプレビュー |
| Sent with your next message: | 随下一条消息发送： | 次のメッセージと一緒に送信します： |
| Don’t Attach the Diagnostics | 不附带连接诊断 | 接続の診断を添付しない |
| I can’t connect to this host. Can you help me find out why? | 这台主机连不上，帮我看看原因。 | このホストに接続できません。原因を調べてください。 |

While the tab is not connected the terminal tools are unavailable, so the assistant can explain the error and what to check in Hatoba but cannot run anything on the host; `references/troubleshooting-connections.md` explains each error kind. If no provider is set up yet, the panel first asks for one (Settings → AI).

## Long pastes

A paste of at least 2,000 characters or 30 lines into the input box (or into the edit box of **Edit and Resend**) does not go into the box. It becomes a chip, "Pasted text · N lines", with an estimate of its tokens (≈N tokens). The text is kept whole, never cut.

- Hover or click the chip to preview the text. The preview has **Paste as Text**, which removes the chip and puts the text into the box at the caret. Its tooltip says that Ctrl+Shift+V (⌘⇧V on macOS) always pastes as text.
- Ctrl+Shift+V (⌘⇧V) in the input box pastes the clipboard as plain text however long it is, without making a chip. A shorter paste with Ctrl+V goes into the box as usual.
- × (**Remove the Pasted Text**) removes the chip.

| en | zh-CN | ja |
|---|---|---|
| Preview the pasted text | 预览粘贴的文本 | 貼り付けたテキストをプレビュー |
| Paste as Text | 作为文本粘贴 | テキストとして貼り付け |
| Remove the Pasted Text | 移除粘贴的文本 | 貼り付けたテキストを外す |

## Text files

The paperclip button in the input area (**Attach Text Files**) opens a file picker for one or more files. Files can also be attached by copying them in a file manager and pasting into the input box, or by dropping them on the panel (a "Drop text files to attach them" overlay shows while dragging; it works in the desktop app too, which reads the dropped paths itself). Dropping needs a provider with a model to be set up. Each file becomes a chip "name · N lines" with a preview and ×.

- Only UTF-8 text files are accepted: source code, configs, logs, Markdown, JSON. A file with NUL bytes or invalid UTF-8 is refused as not a text file. Images (PNG, JPEG, GIF, WebP, SVG and so on) are not supported yet. Documents such as PDF or Word files are binary and are refused too.
- A file over 256 KB is refused. All attachments of one message together (diagnostics, selection, pastes and files) may not pass 512 KB; the one that would pass it is refused. Each refusal shows a message naming the file, for example "<name> is larger than 256 KB, so it can’t be attached." or "A message’s attachments can’t exceed 512 KB in total."
- Only the file's base name is sent, never its folder path, which could reveal a local user name. The text is kept whole, never cut.

| en | zh-CN | ja |
|---|---|---|
| Attach Text Files | 附加文本文件 | テキストファイルを添付 |
| Drop text files to attach them | 松开以附加文本文件 | ドロップしてテキストファイルを添付 |
| Preview the file | 预览文件内容 | ファイルの内容をプレビュー |

## Does it fit the model?

The input area counts the message with its attachments (about 4 characters per token). When the message would bring the conversation past 80% of the model's context window, a note says "With this message the conversation uses about N% of the model’s context. Past 90%, it is compacted before sending." Past 90%, Hatoba compacts the earlier conversation before the request. A message larger than the whole window cannot be sent, because compacting cannot shrink it: the note says "This message is about N tokens, more than the model’s context window… Remove attachments or choose a model with a larger context.", and Send is disabled. The check needs the model's **Context** window to be set (`references/ai-settings.md`).

| en | zh-CN | ja |
|---|---|---|
| The message is larger than the model’s context window | 消息超过了模型的上下文窗口 | メッセージがモデルのコンテキストウィンドウを超えています |
| Context | 上下文窗口 | コンテキスト |

## After sending

A sent message shows each attachment as a collapsed card above what you typed; click a card to open its text. **Export as Markdown…** writes each attachment as a labelled fenced block. **Edit and Resend** keeps the attachments with the message unless their chip is removed, and a long paste in its edit box becomes another attachment. If a message is stopped before it was stored (for example during the compaction before it), its text and its pastes and files return to the input.

| en | zh-CN | ja |
|---|---|---|
| Edit and Resend | 编辑并重新发送 | 編集して再送信 |
| Export as Markdown… | 导出为 Markdown… | Markdown として書き出す… |
