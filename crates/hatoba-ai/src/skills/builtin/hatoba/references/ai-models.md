# Model, thinking level, context and compaction

Which model the assistant uses, how deeply it thinks, and how much of the model's context window a conversation uses. The panel is described in `references/ai-panel.md`; providers and their models are set up in Settings → AI (`references/ai-settings.md`).

## The model

The **Model** button in the input area opens a menu of the models of all providers, grouped by provider, each with its context window. A new conversation starts with the default model (Settings → AI → **Default Model**); a conversation keeps the model it used last. Switching models keeps the whole conversation, but the context count is an estimate until the next response. The first message to the new model gets a divider above it, "Switched to <new model> (was <old model>)", and the model is told that earlier replies came from another model.

Other labels: **Model for new conversations**.

## Thinking level

Under the current model, the same menu has a **Thinking Level** row. It lists **Default** and then only the levels that model offers, lowest first: **Low**, **Medium**, **High**, **Extra High**, **Max**. A model whose levels are unknown (typed in by hand) offers Default, Low, Medium and High.

- **Default** leaves the depth to the provider. It sends no thinking or effort fields, except that for newer Claude models, and for OpenAI reasoning models over the Responses protocol, it turns the display of the reasoning on; a provider that refuses that gets the request again without it.
- A higher level lets the model think longer and use more output tokens, so answers take longer and cost more. Pick a lower level for quick questions.
- The model button shows the level, Default included. When the input area has no room, it leaves out the level, then the model's name, and shows only its cube icon; hover it for both. A level the model lacks is sent as the highest lower level it has (Default when there is none), and the menu shows the level that is actually sent.
- Each conversation keeps the level of its last message, and that syncs to the other devices. A new conversation starts at "Thinking level for new conversations" in Settings → AI (Default unless changed); the levels a model offers are set in the provider's model list (`references/ai-settings.md`).
- If the provider refuses the level fields, Hatoba sends the message again without them and the panel says that the model doesn't accept the chosen level and the message went at its default level (`references/troubleshooting-ai.md`).
- Model reasoning, when the provider returns it, is shown above the answer as **Reasoning**, collapsed.

## Context and compaction

The context meter left of the **Model** button is a ring showing how much of the window the conversation uses (a token count when the model's context window is unknown); hover or click it for the percentage and the numbers. "≈" marks an estimate. At 80% it turns orange and offers **Compact**, and at 90% red: the next message compacts the conversation first; the window comes from the model's **Context** value in the provider.

Other labels: **Context usage**.

- **Compact** (also **Compact Conversation** in the meter's menu, or `/compact` in the input box) asks the model to summarize the conversation. Text after `/compact`, such as `/compact keep the nginx changes`, tells the summary what to keep. `/model` opens the **Model** picker. The summary becomes the start of the context; earlier messages stay visible, marked "Everything above is outside the context".
- Hatoba also compacts by itself before a request that would pass 90% of the context window (the status reads "Compacting the conversation…"). **Stop** during that compaction cancels the message and puts it back in the input. When that happens between tool calls, the summary records where the task stands and the assistant carries on from it.
- A message with large attachments can be too big to fit: see `references/ai-attachments.md`.
