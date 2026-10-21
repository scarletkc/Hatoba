# Troubleshooting: AI assistant, search, skills and MCP

Settings are in `references/ai-settings.md`, the panel in `references/ai-panel.md`, and tools and approvals in `references/ai-tools.md`. The messages are shown exactly as the app prints them in each language. Messages that contain a status code or the provider's own text are paraphrased.

## The provider returned an error

A failed request ends the turn with a banner in the panel ("The request failed", or "The provider returned an error (HTTP N)") and a **Retry** button. The provider's own message follows, untranslated. Typical statuses:

| Status | Usual cause | What to do |
|---|---|---|
| 401, 403 | The API key is wrong, expired or lacks access to the model; for the Anthropic protocol, the wrong **Auth Header** | Edit the provider: **Replace** the key; try the other **Auth Header** |
| 404 | The **Base URL** is wrong (a missing `/v1` for Chat Completions or Responses) or the model ID does not exist | Fix the base URL; check the model ID with **Fetch Models** |
| 400 | The provider rejected the request: the model does not support tool calls or a parameter, or the conversation is too long | Choose a model that supports tools; **Compact** the conversation; check **Max Output** |
| 413, "context length" | The conversation is over the model's context window | **Compact**, or start a **New Conversation**; set the right **Context** value for the model |
| 429 | Rate limit or quota used up | Wait, or switch model or provider |
| 5xx | The provider has a problem | **Retry** later |

Other labels: **Stop**.

Other messages:

- "The response hit the model’s output limit and was cut off."
- "The model declined to respond."
- "Add a model in Settings → AI first."
- "This conversation can’t be opened."
- "History can’t be loaded."
- "The search failed."

- "The response hit the model's output limit…": raise **Max Output** for that model (for the Anthropic protocol it is `max_tokens`) or ask for a shorter answer.
- "The model ended its reply without an answer." under the last reply, with **Retry**: the model sent neither text nor tool calls, at most its reasoning. **Retry** asks again and leaves the empty reply out of the request; if it keeps happening, switch model or choose another **Thinking Level**. Earlier replies without an answer, and a side question's, say "The model gave no answer."
- Models that do not support tool calls cannot work as the assistant: the provider answers with an error as soon as a request carries tools.

Errors of a provider that signs in with a ChatGPT account (usage limits, an ended sign-in) are in `references/ai-chatgpt.md`.

## Thinking level and reasoning

- The note "The model doesn’t accept the chosen thinking level, so this message was sent at its default level." means the provider answered with an error naming the thinking fields, so Hatoba sent the message once more without them. Pick **Default** under **Thinking Level** for that model, or fix the model's levels: in the provider (Settings → AI → Providers), open the model's **Thinking** menu ("Levels this model accepts") and untick the levels it refuses, or choose **None**. A provider that is not really OpenAI- or Anthropic-compatible is the usual cause.
- A level missing from the menu: the menu lists only the levels the model offers. Add them in the model's **Thinking** menu, or use **Fetch Models** so the provider's list fills them in. A model with unknown levels offers Low, Medium and High only.
- A different level than the one chosen: a level the model lacks is sent as the highest lower level it has, and the menu shows the level that is sent.
- No **Reasoning** block: the model may not return its reasoning, or at **Default** it may not think at all (some Claude models only think when a level is chosen): choose a level other than Default. Claude models that write progress notes between tool calls show those notes in the reply instead of any reasoning, at every level, unless their provider refuses that (`references/ai-models.md`). Chat Completions services show reasoning only when they return it as `reasoning_content` or `reasoning`, and Responses services when they return a reasoning summary or reasoning text.
- Slow or costly answers: higher levels think longer and use more output tokens. Lower the level, or set **Thinking level for new conversations** (Settings → AI → Default Model) to Default.

## Test Connection and provider settings

**Test Connection** in the provider form, and in Settings → AI → Web Search, tells the failure apart:

- "The provider rejected the API key. Check the key, and for the Anthropic protocol the auth header."
- "Couldn’t reach the server. Check the base URL and your network."
- "The base URL can’t be used. Check it, and leave out any user name or password."
- "The provider returned an error."
- "Enter a full address that starts with https:// or http://."
- "The base URL can’t contain a user name or password."
- "Enter an API key."
- "Couldn’t load the model list. You can still type model IDs."
- "Enter a number of tokens, such as 200000 or 200K."
- "A model ID can’t be empty."

- "The provider doesn’t know the model …": the model ID has a typo or the provider lacks it. Fix it in **Models**.
- "Couldn’t reach the server": a wrong **Base URL**, no network, or a local server (Ollama, LM Studio) that is not running. For a local server `http://localhost:…` is allowed; any other plain-HTTP address is refused.
- A local server may need the model name exactly as it lists it, and a context window large enough for the conversation.

## Web search

With **Native Search**, the current model searches with its provider's own search: an Anthropic-protocol provider, OpenRouter, OpenAI, xAI or Gemini. Any other OpenAI-compatible provider is tried with OpenAI's web search, and when it turns that away, the assistant's searches report that the provider does not seem to offer web search, without asking it again until Hatoba restarts. With **None** the assistant has no web search tool. A native search that fails reports the provider's error to the assistant: for example, an Anthropic organization whose admin turned web search off in the Claude Console, a model the provider has no search for, or an Anthropic-compatible service that does not run web search ("the model ran no web search"). Choosing Brave Search API, Tavily or SearXNG instead works with every model. In Settings → AI → **Web Search**:

- "The search service rejected the API key."
- "Couldn’t reach the search service. Check the address and your network."
- "The instance URL isn’t valid. Check it and try again."
- "The search service returned an error."
- "Enter the instance URL."

- For **SearXNG** the instance must allow the JSON output format (`search.formats` includes `json` in its settings).
- "Fetch page" refuses addresses on private, loopback or link-local networks, pages that are not text, HTML, JSON or XML, and needs approval in manual mode unless its domain is allowed.

## Skills

- "Use only lowercase letters, digits, and hyphens, up to 64 characters."
- "hatoba is the built-in skill’s name. Choose another."
- "This name now belongs to the built-in skill, so the assistant doesn’t use this skill. Rename it to use it."
- "Another skill already has this name."
- "Enter a description."
- "SKILL.md is over 32 KB. Move detail into other files."
- "A skill can hold at most 5 MB in total."
- "There is no SKILL.md at the top of the folder or .zip."
- "Couldn’t read this folder or .zip."

Other labels: **This skill can’t be imported**, **Skipped Files**.

- A skill name is 1 to 64 lowercase letters, digits and hyphens. The description is at most 1,024 characters; `SKILL.md` and each other file at most 32 KB; at most 200 files and 5 MB per skill; only UTF-8 text files.
- Import needs a `SKILL.md` at the top of the folder or `.zip`, with a valid frontmatter (`name` and `description`). The import dialog lists every problem. If the name exists, choose **Replace it** or **Import under a new name**.
- If the assistant does not use a skill, check that its switch is on and that the description says when to use it.
- The name `hatoba` is reserved for the **Built-in** skill (the first row). Creating a skill with that name is refused, and an import of a skill named `hatoba` asks for another name (it suggests `hatoba-2`). A skill of your own that already had that name stays in the list with the note above, and the assistant does not use it until you rename it or delete it.
- The built-in skill cannot be edited, deleted, exported or replaced. If the assistant cannot answer questions about Hatoba, check that its switch is on (Settings → AI → Skills); the switch syncs across devices.

## MCP servers

- "It failed to start or to answer."
- "Enter the command."
- "The URL can’t contain a user name or password."
- "This looks like a whole command line. Put only the program here, and each argument on its own row."
- "A quote isn’t closed. Check the command."
- "Nothing in this JSON can be imported."

Other labels: **Last lines of stderr**, **Start**.

- A server that fails to start shows **Failed** with the error and the last lines of stderr, in Settings → AI → **MCP Servers** (click its row) and in the panel's **MCP Tools** (+ menu) or `/mcp`; while a server the conversation uses has failed, a red line above the input box names it. Its tool calls return errors meanwhile. Read the stderr, fix the command or the environment, then **Restart**.
- Command servers are started like a shell would: `npx` finds `npx.cmd` on Windows. Put only the program in **Command** and each argument on its own row; if the whole command line was pasted, use **Split Into Arguments**. A missing program or a wrong `PATH` is the usual cause of a failed start.
- Servers that need OAuth sign-in are not supported; use a token in **Headers** if the service allows it. When a server with an `Authorization` header fails with "the token is invalid or has expired (HTTP 401)", **Replace** the header's value with a valid token; a token pasted without a scheme is saved with `Bearer ` in front of it.
- Servers start when a conversation first needs their tools and stop on quit or lock; after unlocking they start again when needed. Opening the panel's `/` or `@` menu starts them too, for their prompts and resources. A server switched off for this device, or for the conversation in **MCP Tools** or `/mcp`, offers no tools, prompts or resources.
- A server added on another device arrives off if it is a Command server, and a Command server turns off here when its command, arguments or environment variable names change on another device (its **Always allow** is cleared too); switch it on here after checking the command.

## Still not solved

If the problem is not covered by the troubleshooting files and looks like a bug, use the feedback path in SKILL.md ("Project, feedback and newer information"): **Report a Problem** in Settings → About opens GitHub's bug report form. Offer to draft the text (provider type and protocol, model, the HTTP status and the error text, the version), and remind the user to remove API keys, tokens, private URLs and hostnames before sending. The user submits it.
