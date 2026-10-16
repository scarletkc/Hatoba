# Settings → AI

Open Settings (the gear in the sidebar, or Ctrl+, / ⌘,) and choose the **AI** tab. It has these sections in order: Providers, Default Model, Web Search, Custom Instructions (`references/ai-instructions.md`), Terminal Screen, Memory (`references/ai-instructions.md`), Skills, MCP Servers, Auto Mode and This Device. Using the assistant is covered in `references/ai-panel.md`.

## Providers

The assistant calls the providers added here directly from the device, with the user's own API keys. Nothing is sent until a provider is added and a message is sent. The provider must support tool calls. Local servers such as Ollama and LM Studio work.

Other labels: **Name**, **Clear**, **Model ID**, **Display Name**.

- **Continue with ChatGPT**, beside **Add Provider**, adds a provider that signs in with a ChatGPT account and uses its ChatGPT plan instead of an API key: see `references/ai-chatgpt.md`.
- **Add Provider** (shown at the top once a provider exists, or in the empty state) opens the form; click a provider's row to edit it, and the bin icon deletes it. Deleting removes its API key and model list from the vault and from the other devices; existing conversations stay but need another model to continue.
- **Protocol**: **OpenAI Chat Completions** (OpenAI, Gemini through its OpenAI-compatible endpoint, local servers, most other services), **OpenAI Responses** (OpenAI's newer API, also offered by xAI, OpenRouter and some local servers) or **Anthropic Messages** (Claude and vendors with an Anthropic-compatible endpoint). The form picks Responses for `api.openai.com`, and Anthropic for `api.anthropic.com` and for base URLs ending in `/anthropic`. **System One** is only for the judge of Auto mode (below): its models answer the judge's questions and are never offered for conversations. The form picks it for `api.typesafe.ai`, base URLs ending in `/systemone` and Workers AI addresses with `/ai/run`, and hides Context, Max Output, Thinking and Auth Header for it.
- **Base URL**: for Chat Completions and Responses include the API version, such as `https://api.openai.com/v1`. For Anthropic use the address the vendor documents for Anthropic SDKs, such as `https://api.anthropic.com`. For System One: TypeSafe `https://api.typesafe.ai/v1` (model `jev-latest`), OpenRouter `https://openrouter.ai/api/v1`, or Cloudflare Workers AI `https://api.cloudflare.com/client/v4/accounts/<account ID>/ai/run` (`@cf/cloudflare/clef` or `@cf/cloudflare/clef-flash`). Plain HTTP works too.
- **API Key**: stored encrypted in the vault and synced to the other devices. After saving it shows **Saved** and can be replaced or cleared but never viewed. It may stay empty for a server that needs none.
- **Auth Header** (Anthropic protocol only): `x-api-key` (most services) or `Authorization: Bearer` (a few).
- **Models**: type a model ID and press Enter (or **Add**), or use **Fetch Models** to list the provider's models and pick from them. Each model has a display name, an optional **Context** window and **Max Output** in tokens (enter 200000 or 200K). The context window drives the usage meter and compaction. **Thinking** lists the thinking levels the model accepts (Low, Medium, High, Extra High, Max). Its button opens a menu titled "Levels this model accepts" where each level is ticked or not; **None** means the model offers only Default. A model typed in by hand, or one whose list gave no levels, shows Low, Medium and High. **Fetch Models** fills the levels in from the provider's list when it gives them (Anthropic's does), also for models already in the form that have none yet. The output limit becomes `max_tokens` for Anthropic requests (16,000 when unknown).
- **Test Connection** sends a minimal request to the first model (or fetches the model list when none is entered) and tells a rejected key, an unreachable server and an unknown model apart. Test messages are in `references/troubleshooting-ai.md`.

## Default Model and Web Search

- **Model for new conversations** is the model a new conversation starts with. It lists the models of every provider but System One ones, and it syncs. Each conversation then keeps the model it used last.
- **Thinking level for new conversations** is the level a new conversation starts at: Default, Low, Medium, High, Extra High or Max (see `references/ai-models.md`). It syncs. A model that does not offer that level uses the highest lower level it has. Each conversation then keeps the level it used last. What is sent: for OpenAI Chat Completions a `reasoning_effort` of low, medium or high (xhigh or max only for a model whose levels include them); for OpenAI Responses a `reasoning` effort with the same values, asking for the reasoning summarized; for Anthropic Messages an `output_config` effort, with adaptive thinking switched on for models that support it. Default sends none of these, except that models that reason anyway (newer Claude models, and GPT-5 and o-series models over Responses) are asked for their reasoning so the panel can show it.
- **Search provider** is the backend of the assistant's web search. **Native Search** (the default) has the current model search with its own provider's search, which costs what that provider charges for a search and for the model's tokens: Anthropic's web search on any Anthropic-protocol provider, OpenRouter's web search server tool (the model's own provider's search where it has one, Exa otherwise), OpenAI's and xAI's web search through their Responses API, and Grounding with Google Search on Gemini. Any other OpenAI-compatible provider, such as a relay of OpenAI's API, is tried with OpenAI's web search; one that turns it away is not asked again until Hatoba restarts, and the assistant's searches then report that it has no native search. Or choose **Brave Search API** or **Tavily** (enter the **API Key**) or **SearXNG** (enter the **Instance URL** of your own instance, which must have the JSON format enabled), then **Test Connection** and **Save**. **None** means the assistant has no web search tool. Search terms go to the chosen service, or with Native Search to the model's provider. **Remove Configuration** deletes the saved settings and key.

## Terminal Screen

**Send with Each Message by Default** (on by default, synced) shows the assistant the attached terminal's screen with every message, in each conversation where **Send Terminal Screen** in the input area's + menu was not changed; see `references/ai-attachments.md`.

## Skills

A skill is a set of instructions for the assistant in the Agent Skills format: a `SKILL.md` with a name and a description, and optional text files such as `topics/nginx.md`. The assistant sees the name and description of every enabled skill and reads the rest with its **Read skill** tool when it needs it (no approval needed). Skills contain text only. Hatoba never runs a file from a skill, and `allowed-tools` changes nothing. Skills sync to the other devices. Skill text reaches the model as instructions, so import only skills you trust.

Other labels: **Description**, **Files to Save**, **Replace Skill**.

- **New Skill** writes one in the app: a name (1 to 64 lowercase letters, digits and hyphens), a description (at most 1,024 characters; tells the assistant what the skill does and when to use it), the **Instructions** (the Markdown body of `SKILL.md`, at most 32 KB) and extra files added with **Add File** (relative paths, each at most 32 KB; at most 200 files and 5 MB in all).
- **Import** has **Import Folder…** and **Import .zip…** for a folder or archive with a `SKILL.md` at its top. **Import Skill** shows the skill's name and description and lists every file to be saved before anything is stored; click a file to see its text. Files that are not UTF-8 text are skipped and listed, and a file over 32 KB or too large to sync once encoded is refused. If the name is taken, choose **Replace it** or **Import under a new name**.
- Each row of your own skills has an enable switch, an export button (saves a `.zip`), and a delete button. Click the row to edit it.

*The built-in skill.* The first row is always the `hatoba` skill that ships with the app (this documentation). It carries a **Built-in** badge, its description and file count, an enable switch, and a view button (an eye icon, labelled "View hatoba"; clicking the row does the same) that opens a read-only viewer titled **Built-in skill** with `SKILL.md` and every other file. It is on by default, and the switch syncs to the other devices; switched off, the assistant no longer sees it and cannot answer from it. It comes with the app and changes with each version, so it cannot be edited, deleted, exported or replaced.

- The name `hatoba` is reserved. **New Skill** refuses it with "hatoba is the built-in skill’s name. Choose another." An import of a skill named `hatoba` cannot replace anything: the import dialog says the name is the built-in skill's and asks for another one (it suggests `hatoba-2`).
- A skill of your own that was saved under that name by an earlier version stays in the list with the note "This name now belongs to the built-in skill, so the assistant doesn’t use this skill. Rename it to use it." Rename it, or delete it.

## MCP Servers

Adding a Command or HTTP server, templates, **Always ask**, import and export, and each server's state and tools are in `references/ai-mcp.md`.

## Auto Mode

The judge of Auto mode (`references/ai-tools.md`). It syncs.

Other labels: **Choose a judge**, **Run without asking**, **Ask**, **As expected**, **Not as expected**.

- **Judge model** lists the models of every provider, System One ones included, and **None**. Auto can't be chosen until a judge is. It can be a System One model (TypeSafe's Jev, Cloudflare's Clef) or a chat model.
- **Test** asks the judge about thirteen sample calls, such as checking disk space, deleting logs nobody asked to delete, fetching a URL that carries a token or typing `ls -a` at the terminal's shell prompt, and runs none of them. It shows how many came out as expected; **Details** shows each one's expected decision, the judge's and its answers. A sample that ran when it should have asked is red; one the judge did not answer shows **No answer** and why.
- What the judge receives is listed in `references/ai-tools.md`; a judge on another service than the conversation's model receives it too.

## This Device

These settings stay on this device and are not synced.

Other labels: **Turn On Bypass**.

- **Default Permission Mode**: **Manual approval** (default), **Auto** or **Bypass**. New conversations start in it. Auto can't be chosen until a judge is (above); choosing Bypass the first time asks for confirmation.
- **Saved Approvals**: what **Always Allow** on an approval card saved (`references/ai-tools.md`): commands starting with a prefix and the typing, edit and write tools, each for a host or quick connection, and page domains for every tab (a page that redirects to another domain is not followed; fetching that domain needs its own approval). They are sealed in the vault and kept until **Remove** (×) deletes one. A command's prefix is its name, plus the subcommand for tools like `git` or `systemctl` ("git pull"); the folder the assistant runs a command in is allowed by itself ("cd C:\src") where moving there needs approval, as in Command Prompt; a command line runs without asking when each of its commands is allowed or only reads, and `sudo`, `sh -c`, redirection, `$` or backticks always ask. Interpreters (`python3` …), `rm`, `dd`, `chmod`, `kill`, `curl`, `scp` and the like, also as `/bin/rm` or `python3.12`, get Allow for This Conversation only. Allow for This Conversation covers the same, until the app quits. An MCP tool's **Always allow** is its server's setting (`references/ai-mcp.md`).
