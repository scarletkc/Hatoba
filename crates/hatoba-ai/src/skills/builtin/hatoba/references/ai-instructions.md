# Custom instructions, host notes and memory

Two boxes tell the assistant what it should know in every request, without repeating it in each message: **Custom Instructions** in Settings → AI, for every conversation, and **AI Notes** in the host editor, for conversations on one host. **Memory** is what the assistant notes for itself across conversations (below). Both go into the system prompt of every request, so they reach the model provider and use part of the context window each time. Skills (`references/ai-settings.md`) suit long procedures better, because the assistant reads a skill only when a request needs it.

## Custom Instructions (Settings → AI)

Settings → AI → **Custom Instructions**, between Web Search and Skills. Good for preferences that hold everywhere: the language to answer in, the tone, how much to explain, the operating systems, shells and tools you use.

- Up to 32 KB of text (UTF-8 bytes), multi-line. Below the box: the size in bytes and an estimate of the tokens it adds to every request (about 4 characters a token). Text over the limit turns red, says how many bytes are allowed, and is not saved.
- It saves by itself a moment after you stop typing, when the box loses focus, and when you switch to another Settings tab or close Settings. There is no Save button.
- It syncs to your other devices with the settings.
- Every request carries it, in every conversation and in Compact.

## AI Notes (host editor)

The host editor (`references/hosts-and-connecting.md`) has an **AI Notes** section after **Notes**. Good for what the assistant should know about that host: what runs there, important paths and services, what to avoid (for example "never restart postgresql during business hours").

- Up to 16 KB, with the same size and token estimate. Text over the limit keeps the host from being saved until it is shortened.
- The notes are part of the host: they save with **Save**, sync with the host, and are copied by **Duplicate** in the host's row menu. The host's **Notes** section is for you and never goes to the assistant; **AI Notes** does.
- Every request of a conversation on that host carries them: the terminal tab's host, or the conversation's host on the home tab. When a conversation moves to another host, the next requests carry the new host's notes.

## Memory (Settings → AI)

The assistant keeps short notes for later conversations, as Claude Code's auto memory does: what you ask it to remember ("remember that web-1 uses doas"), your corrections and preferences, and facts about your hosts and setup that it could not find by looking. Custom instructions and host notes are what you write; memories are what the assistant saves, and you can read and change them.

- Each memory has a name and a few lines of text. All memories together, names included, hold up to 16 KB (UTF-8 bytes). When they are full, the assistant has to merge or delete some before it saves another.
- The assistant saves, replaces and deletes memories with its **Memory** tool, which runs without asking in every permission mode; the call shows in the conversation like any other tool.
- Every new conversation starts with all memories, oldest first. A memory saved during a conversation reaches new conversations, and the same conversation after it is compacted (`references/ai-models.md`).
- They are not for passwords, keys or tokens: like custom instructions, they go to the model provider.
- Where a memory differs from your custom instructions or a host's AI notes, those win.
- Settings → AI → **Memory** (or `/memory` in the input box) lists them with the space they take. Click one to edit it, the trash button deletes it, **Add Memory** adds one, and **Delete All** deletes every memory. **Remember across conversations** turns memory off: the assistant then neither sees nor saves memories, and the saved ones are kept.
- Memories sync to your other devices with the vault, and so does the switch.

## How the assistant uses them

The assistant follows custom instructions and host notes, and where they differ from Hatoba's default rules (answering in your language, briefly, in Markdown, and so on), they win. They cannot change approvals: in manual mode, a call that asks still waits for you, whatever the text says. They are instructions, so write only what you mean the model to do, and leave out passwords, keys and tokens, since the text goes to the model provider.

If the assistant seems to ignore them, check that the text was saved (reopen Settings → AI or the host), that the conversation is on the host whose notes you edited, and whether the model is small or the instructions are long or contradict each other.
