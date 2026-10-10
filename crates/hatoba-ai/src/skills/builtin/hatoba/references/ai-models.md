# Model, thinking level, context and compaction

Which model the assistant uses, how deeply it thinks, and how much of the model's context window a conversation uses. The panel is described in `references/ai-panel.md`; providers and their models are set up in Settings → AI (`references/ai-settings.md`).

## The model

The **Model** button in the input area opens a menu of the models of all providers, grouped by provider, each with its context window. A new conversation starts with the default model (Settings → AI → **Default Model**); a conversation keeps the model it used last. Switching models keeps the whole conversation, but the context count is an estimate until the next response. The first message to the new model gets a divider above it, "Switched to <new model> (was <old model>)", and the model is told that earlier replies came from another model.

| en | zh-CN | ja |
|---|---|---|
| Model | 模型 | モデル |
| Default Model | 默认模型 | 既定のモデル |
| Model for new conversations | 新对话使用的模型 | 新しい会話で使うモデル |

## Thinking level

Under the current model, the same menu has a **Thinking Level** row. It lists **Default** and then only the levels that model offers, lowest first: **Low**, **Medium**, **High**, **Extra High**, **Max**. A model whose levels are unknown (typed in by hand) offers Default, Low, Medium and High.

| en | zh-CN | ja |
|---|---|---|
| Thinking Level | 思考程度 | 思考レベル |
| Default | 默认 | 既定 |
| Low | 低 | 低 |
| Medium | 中 | 中 |
| High | 高 | 高 |
| Extra High | 超高 | 超高 |
| Max | 最高 | 最大 |

- **Default** sends no thinking or effort fields, so every provider accepts it and the depth is left to the provider. For newer Claude models it only turns the display of the reasoning on.
- A higher level lets the model think longer and use more output tokens, so answers take longer and cost more. Pick a lower level for quick questions.
- The model button shows the level when it is not Default. A level the model lacks is sent as the highest lower level it has (Default when there is none), and the menu shows the level that is actually sent.
- Each conversation keeps the level of its last message, and that syncs to the other devices. A new conversation starts at "Thinking level for new conversations" in Settings → AI (Default unless changed); the levels a model offers are set in the provider's model list (`references/ai-settings.md`).
- If the provider refuses the level fields, Hatoba sends the message again without them and the panel says that the model doesn't accept the chosen level and the message went at its default level (`references/troubleshooting-ai.md`).
- Model reasoning, when the provider returns it, is shown above the answer as **Reasoning**, collapsed.

| en | zh-CN | ja |
|---|---|---|
| Reasoning | 思考过程 | 思考過程 |

## Context and compaction

The context meter at the right of the input area is a ring with a percentage (or a token count when the model's context window is unknown) showing how much of the window the conversation uses; hover for the numbers. "≈" marks an estimate. At 80% it turns to a warning color and offers **Compact**; the window comes from the model's **Context** value in the provider.

| en | zh-CN | ja |
|---|---|---|
| Context usage | 上下文用量 | コンテキストの使用量 |
| Compact | 压缩 | 圧縮 |
| Compact Conversation | 压缩对话 | 会話を圧縮 |
| Everything above is outside the context | 以上内容不在上下文中 | ここより上はコンテキストに含まれません |
| Context | 上下文窗口 | コンテキスト |

- **Compact** (also **Compact Conversation** in the meter's menu) asks the model to summarize the conversation. The summary becomes the start of the context; earlier messages stay visible, marked "Everything above is outside the context".
- Hatoba also compacts by itself before a request that would pass 90% of the context window (the status reads "Compacting the conversation…"). **Stop** during that compaction cancels the message and puts it back in the input. When that happens between tool calls, the summary records where the task stands and the assistant carries on from it.
- A message with large attachments can be too big to fit: see `references/ai-attachments.md`.

| en | zh-CN | ja |
|---|---|---|
| Stop | 停止 | 停止 |
