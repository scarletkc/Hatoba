# Custom instructions and host notes

Two boxes tell the assistant what it should know in every request, without repeating it in each message: **Custom Instructions** in Settings → AI, for every conversation, and **AI Notes** in the host editor, for conversations on one host. Both go into the system prompt of every request, so they reach the model provider and use part of the context window each time. Skills (`references/ai-settings.md`) suit long procedures better, because the assistant reads a skill only when a request needs it.

| en | zh-CN | ja |
|---|---|---|
| Custom Instructions | 自定义指令 | カスタム指示 |
| AI Notes | AI 备注 | AI 向けメモ |

## Custom Instructions (Settings → AI)

Settings → AI → **Custom Instructions**, between Web Search and Skills. Good for preferences that hold everywhere: the language to answer in, the tone, how much to explain, the operating systems, shells and tools you use.

- Up to 4,000 characters, multi-line. Below the box: the character count and an estimate of the tokens it adds to every request (about 4 characters a token). Text over the limit turns red, says "At most 4,000 characters. Shorten the text to save it.", and is not saved.
- It saves by itself a moment after you stop typing, when the box loses focus, and when you switch to another Settings tab or close Settings. There is no Save button.
- It syncs to your other devices with the settings.
- Every request carries it, in every conversation and in Compact.

## AI Notes (host editor)

The host editor (`references/hosts-and-connecting.md`) has an **AI Notes** section after **Notes**. Good for what the assistant should know about that host: what runs there, important paths and services, what to avoid (for example "never restart postgresql during business hours").

- Up to 2,000 characters, with the same count and token estimate. Text over the limit keeps the host from being saved until it is shortened.
- The notes are part of the host: they save with **Save**, sync with the host, and are copied by **Duplicate** in the host's row menu. The host's **Notes** section is for you and never goes to the assistant; **AI Notes** does.
- Every request of a conversation on that host carries them: the terminal tab's host, or the conversation's host on the home tab. When a conversation moves to another host, the next requests carry the new host's notes.

## How the assistant uses them

The assistant follows custom instructions and host notes unless they conflict with Hatoba's own rules. A language, tone or format they ask for replaces the defaults (answering in your language, briefly, in Markdown). They cannot change approvals: in manual mode, commands still wait for your approval, whatever the text says. They are instructions, so write only what you mean the model to do, and leave out passwords, keys and tokens, since the text goes to the model provider.

If the assistant seems to ignore them, check that the text was saved (reopen Settings → AI or the host), that the conversation is on the host whose notes you edited, and whether the model is small or the instructions are long or contradict each other.
