# AI side questions

A side question asks the assistant about the conversation without stopping it: a running turn, its tools and its approval card carry on untouched. It also works when nothing is running, but not while the conversation is being compacted, by **Compact** or automatically. The conversation needs at least one message sent first.

## Asking one

- **Ask a Side Question** (the speech bubble beside **Send** or **Stop** in the input area), or Alt+Enter (⌥↩ on macOS), asks what the input box holds, with its attachments (`references/ai-attachments.md`). Typing `/btw <question>` and Enter asks it too. While a turn runs, plain Enter queues a message for when the turn ends, and Ctrl+Enter (⌘↩) sends it at once, stopping the turn (`references/ai-panel.md`); the input box says so.
- If the side question is refused, fails or is stopped, its text and attachments go back to the input area: the text only if the box is empty again.
- **Ask About This** on an approval card opens a box on the card with a suggested question about the call; **Ask** sends it, and Esc closes the box. The call is neither run nor rejected, and the card keeps waiting for **Run** or **Reject**. It is not offered while you edit the call, since the assistant sees the call as the model wrote it.
- The assistant answers from the conversation so far, with the conversation's model, and uses no tools; it starts no MCP server. It is told which calls of the running turn still wait for approval or are still running. If the model tries to use a tool anyway, Hatoba asks it once more without tools.

| en | zh-CN | ja |
|---|---|---|
| Ask a Side Question | 顺便问一句 | ついでに質問 |
| Ask About This | 先问一下 | これについて質問 |
| Ask | 提问 | 質問する |

## The answer

The answer streams into a bubble with a dashed border at the end of the conversation, titled "Side question" and marked "Not saved", with **Copy** and a close button. A failed side question shows the error and **Retry**.

| en | zh-CN | ja |
|---|---|---|
| Side question | 顺便一问 | ついでの質問 |
| Not saved | 不会保存 | 保存されません |

- Nothing of it is stored, synced, exported or searched, and later requests leave it out, so the assistant does not remember it in the conversation. To keep something from it, send it as a normal message.
- Closing the bubble, asking another side question, sending a message (**Edit and Resend**, and **Retry** after a failed reply, too) or opening another conversation in the panel discards it; an answer still coming stops first.
- The stop button on the bubble, or Esc in the panel while the answer comes, stops only that side question; Esc again stops the turn. Stopping the turn leaves the side question running, and locking the vault stops it.
- A side question is a request to the model provider like any other, so it uses tokens; the provider's prompt cache usually serves most of the conversation.
