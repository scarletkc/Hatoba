# Settings → AI

Open Settings (the gear in the sidebar, or Ctrl+, / ⌘,) and choose the **AI** tab. It has these sections in order: Providers, Default Model, Web Search, Custom Instructions (`references/ai-instructions.md`), Terminal Screen, Skills, MCP Servers, Auto Mode and This Device. Using the assistant is covered in `references/ai-panel.md`.

| en | zh-CN | ja |
|---|---|---|
| Settings | 设置 | 設定 |
| AI | AI | AI |
| Providers | 服务商 | プロバイダー |
| Default Model | 默认模型 | 既定のモデル |
| Web Search | 网页搜索 | Web 検索 |
| Custom Instructions | 自定义指令 | カスタム指示 |
| Terminal Screen | 终端屏幕 | ターミナル画面 |
| Skills | 技能 | スキル |
| MCP Servers | MCP 服务器 | MCP サーバー |
| Auto Mode | 自动批准模式 | 自動承認モード |
| This Device | 此设备 | このデバイス |

## Providers

The assistant calls the providers added here directly from the device, with the user's own API keys. Nothing is sent until a provider is added and a message is sent. The provider must support tool calls. Local servers such as Ollama and LM Studio work.

| en | zh-CN | ja |
|---|---|---|
| Add Provider | 添加服务商 | プロバイダーを追加 |
| Name | 名称 | 名前 |
| Protocol | 协议 | プロトコル |
| OpenAI Chat Completions | OpenAI Chat Completions | OpenAI Chat Completions |
| Anthropic Messages | Anthropic Messages | Anthropic Messages |
| System One | System One | System One |
| Base URL | 基础 URL | ベース URL |
| API Key | API 密钥 | API キー |
| Auth Header | 认证头 | 認証ヘッダー |
| Test Connection | 测试连接 | 接続をテスト |
| Save | 保存 | 保存 |
| Saved | 已保存 | 保存済み |
| Replace | 替换 | 置き換え |
| Clear | 清除 | 削除 |
| Models | 模型 | モデル |
| Fetch Models | 获取模型列表 | モデル一覧を取得 |
| Model ID | 模型 ID | モデル ID |
| Display Name | 显示名称 | 表示名 |
| Context | 上下文窗口 | コンテキスト |
| Max Output | 输出上限 | 出力上限 |
| Thinking | 思考程度 | 思考レベル |
| Levels this model accepts | 这个模型接受的思考程度 | このモデルが受け付けるレベル |
| None | 无 | なし |
| Add | 添加 | 追加 |

- **Add Provider** (shown at the top once a provider exists, or in the empty state) opens the form; click a provider's row to edit it, and the bin icon deletes it. Deleting removes its API key and model list from the vault and from the other devices; existing conversations stay but need another model to continue.
- **Protocol**: **OpenAI Chat Completions** (OpenAI, Gemini through its OpenAI-compatible endpoint, local servers, most other services) or **Anthropic Messages** (Claude and vendors with an Anthropic-compatible endpoint). The form picks Anthropic for `api.anthropic.com` and for base URLs ending in `/anthropic`. **System One** is only for the judge of Auto mode (below): its models answer the judge's questions and are never offered for conversations. The form picks it for `api.typesafe.ai`, base URLs ending in `/systemone` and Workers AI addresses with `/ai/run`, and hides Context, Max Output, Thinking and Auth Header for it.
- **Base URL**: for Chat Completions include the API version, such as `https://api.openai.com/v1`. For Anthropic use the address the vendor documents for Anthropic SDKs, such as `https://api.anthropic.com`. For System One: TypeSafe `https://api.typesafe.ai/v1` (model `jev-latest`), OpenRouter `https://openrouter.ai/api/v1`, or Cloudflare Workers AI `https://api.cloudflare.com/client/v4/accounts/<account ID>/ai/run` (`@cf/cloudflare/clef` or `@cf/cloudflare/clef-flash`). Plain HTTP works too.
- **API Key**: stored encrypted in the vault and synced to the other devices. After saving it shows **Saved** and can be replaced or cleared but never viewed. It may stay empty for a server that needs none.
- **Auth Header** (Anthropic protocol only): `x-api-key` (most services) or `Authorization: Bearer` (a few).
- **Models**: type a model ID and press Enter (or **Add**), or use **Fetch Models** to list the provider's models and pick from them. Each model has a display name, an optional **Context** window and **Max Output** in tokens (enter 200000 or 200K). The context window drives the usage meter and compaction. **Thinking** lists the thinking levels the model accepts (Low, Medium, High, Extra High, Max). Its button opens a menu titled "Levels this model accepts" where each level is ticked or not; **None** means the model offers only Default. A model typed in by hand, or one whose list gave no levels, shows Low, Medium and High. **Fetch Models** fills the levels in from the provider's list when it gives them (Anthropic's does), also for models already in the form that have none yet. The output limit becomes `max_tokens` for Anthropic requests (16,000 when unknown).
- **Test Connection** sends a minimal request to the first model (or fetches the model list when none is entered) and tells a rejected key, an unreachable server and an unknown model apart. Test messages are in `references/troubleshooting-ai.md`.

## Default Model and Web Search

| en | zh-CN | ja |
|---|---|---|
| Model for new conversations | 新对话使用的模型 | 新しい会話で使うモデル |
| Thinking level for new conversations | 新对话的思考程度 | 新しい会話の思考レベル |
| Search provider | 搜索服务 | 検索プロバイダー |
| Brave Search API | Brave Search API | Brave Search API |
| Tavily | Tavily | Tavily |
| SearXNG | SearXNG | SearXNG |
| Instance URL | 实例地址 | インスタンスの URL |
| Remove Configuration | 移除配置 | 設定を削除 |

- **Model for new conversations** is the model a new conversation starts with. It lists the models of every provider but System One ones, and it syncs. Each conversation then keeps the model it used last.
- **Thinking level for new conversations** is the level a new conversation starts at: Default, Low, Medium, High, Extra High or Max (see `references/ai-models.md`). It syncs. A model that does not offer that level uses the highest lower level it has. Each conversation then keeps the level it used last. What is sent: for OpenAI Chat Completions a `reasoning_effort` of low, medium or high (xhigh or max only for a model whose levels include them); for Anthropic Messages an `output_config` effort, with adaptive thinking switched on for models that support it. Default sends none of these.
- **Search provider** is the backend of the assistant's web search. Choose **Brave Search API** or **Tavily** (enter the **API Key**) or **SearXNG** (enter the **Instance URL** of your own instance, which must have the JSON format enabled), then **Test Connection** and **Save**. **None** (the default) means the assistant has no web search tool. Search terms go to the chosen service. **Remove Configuration** deletes the saved settings and key.

## Terminal Screen

**Send with Each Message by Default** (on by default, synced) shows the assistant the attached terminal's screen with every message, in each conversation where **Send Terminal Screen** in the input area's + menu was not changed; see `references/ai-attachments.md`.

| en | zh-CN | ja |
|---|---|---|
| Send with Each Message by Default | 默认随每条消息发送 | 既定でメッセージごとに送信 |

## Skills

A skill is a set of instructions for the assistant in the Agent Skills format: a `SKILL.md` with a name and a description, and optional text files such as `topics/nginx.md`. The assistant sees the name and description of every enabled skill and reads the rest with its **Read skill** tool when it needs it (no approval needed). Skills contain text only. Hatoba never runs a file from a skill, and `allowed-tools` changes nothing. Skills sync to the other devices. Skill text reaches the model as instructions, so import only skills you trust.

| en | zh-CN | ja |
|---|---|---|
| Read skill | 读取技能 | スキルを読む |
| New Skill | 新建技能 | スキルを作成 |
| Import | 导入 | インポート |
| Import Folder… | 导入文件夹… | フォルダーをインポート… |
| Import .zip… | 导入 .zip… | .zip をインポート… |
| Description | 描述 | 説明 |
| Instructions | 说明 | 指示 |
| Files | 文件 | ファイル |
| Add File | 添加文件 | ファイルを追加 |
| Import Skill | 导入技能 | スキルをインポート |
| Files to Save | 将保存的文件 | 保存されるファイル |
| Replace it | 替换它 | 置き換える |
| Import under a new name | 用新名称导入 | 新しい名前でインポート |
| Replace Skill | 替换技能 | スキルを置き換え |

- **New Skill** writes one in the app: a name (1 to 64 lowercase letters, digits and hyphens), a description (at most 1,024 characters; tells the assistant what the skill does and when to use it), the **Instructions** (the Markdown body of `SKILL.md`, at most 32 KB) and extra files added with **Add File** (relative paths, each at most 32 KB; at most 200 files and 5 MB in all).
- **Import** has **Import Folder…** and **Import .zip…** for a folder or archive with a `SKILL.md` at its top. **Import Skill** shows the skill's name and description and lists every file to be saved before anything is stored; click a file to see its text. Files that are not UTF-8 text are skipped and listed, and a file over 32 KB or too large to sync once encoded is refused. If the name is taken, choose **Replace it** or **Import under a new name**.
- Each row of your own skills has an enable switch, an export button (saves a `.zip`), and a delete button. Click the row to edit it.

*The built-in skill.* The first row is always the `hatoba` skill that ships with the app (this documentation). It carries a **Built-in** badge, its description and file count, an enable switch, and a view button (an eye icon, labelled "View hatoba"; clicking the row does the same) that opens a read-only viewer titled **Built-in skill** with `SKILL.md` and every other file. It is on by default, and the switch syncs to the other devices; switched off, the assistant no longer sees it and cannot answer from it. It comes with the app and changes with each version, so it cannot be edited, deleted, exported or replaced.

| en | zh-CN | ja |
|---|---|---|
| Built-in | 内置 | 組み込み |
| Built-in skill | 内置技能 | 組み込みスキル |
| hatoba is the built-in skill’s name. Choose another. | hatoba 是内置技能的名称，请换一个。 | hatoba は組み込みスキルの名前です。別の名前にしてください。 |
| This name now belongs to the built-in skill, so the assistant doesn’t use this skill. Rename it to use it. | 这个名称现在属于内置技能，助手不会使用这个技能。重命名后即可使用。 | この名前は組み込みスキルのものになったため、アシスタントはこのスキルを使いません。名前を変更すると使えるようになります。 |

- The name `hatoba` is reserved. **New Skill** refuses it with "hatoba is the built-in skill’s name. Choose another." An import of a skill named `hatoba` cannot replace anything: the import dialog says the name is the built-in skill's and asks for another one (it suggests `hatoba-2`).
- A skill of your own that was saved under that name by an earlier version stays in the list with the note "This name now belongs to the built-in skill, so the assistant doesn’t use this skill. Rename it to use it." Rename it, or delete it.

## MCP Servers

Adding a Command or HTTP server, templates, **Always ask**, import and export, and each server's state and tools are in `references/ai-mcp.md`.

## Auto Mode

The judge of Auto mode (`references/ai-tools.md`). It syncs.

| en | zh-CN | ja |
|---|---|---|
| Auto Mode | 自动批准模式 | 自動承認モード |
| Judge model | 判定模型 | 判定モデル |
| Choose a judge | 选择判定模型 | 判定モデルを選択 |
| Test | 测试 | テスト |
| Details | 详情 | 詳細 |
| Run without asking | 直接运行 | 確認せずに実行 |
| Ask | 询问 | 確認する |
| As expected | 符合预期 | 期待どおり |
| Not as expected | 不符合预期 | 期待と異なる |
| No answer | 没有回答 | 回答なし |

- **Judge model** lists the models of every provider, System One ones included, and **None**. Auto can't be chosen until a judge is. It can be a System One model (TypeSafe's Jev, Cloudflare's Clef) or a chat model.
- **Test** asks the judge about thirteen sample calls, such as checking disk space, deleting logs nobody asked to delete, fetching a URL that carries a token or typing `ls -a` at the terminal's shell prompt, and runs none of them. It shows how many came out as expected; **Details** shows each one's expected decision, the judge's and its answers. A sample that ran when it should have asked is red; one the judge did not answer shows **No answer** and why.
- What the judge receives is listed in `references/ai-tools.md`; a judge on another service than the conversation's model receives it too.

## This Device

These settings stay on this device and are not synced.

| en | zh-CN | ja |
|---|---|---|
| Default Permission Mode | 默认权限模式 | 既定の権限モード |
| Manual approval | 手动批准 | 手動承認 |
| Auto | 自动批准 | 自動承認 |
| Bypass | 绕过批准 | 承認をバイパス |
| Turn On Bypass | 开启绕过模式 | バイパスをオンにする |
| Saved Approvals | 已保存的批准 | 保存済みの許可 |
| Remove | 移除 | 削除 |

- **Default Permission Mode**: **Manual approval** (default), **Auto** or **Bypass**. New conversations start in it. Auto can't be chosen until a judge is (above); choosing Bypass the first time asks for confirmation.
- **Saved Approvals**: what **Always Allow** on an approval card saved (`references/ai-tools.md`): commands starting with a prefix and the typing, edit and write tools, each for a host or quick connection, and page domains for every tab (a page that redirects to another domain is not followed; fetching that domain needs its own approval). They are sealed in the vault and kept until **Remove** (×) deletes one. A command's prefix is its name, plus the subcommand for tools like `git` or `systemctl` ("git pull"); a command line runs without asking when each of its commands is allowed or only reads, and `sudo`, `sh -c`, redirection, `$` or backticks always ask. Interpreters (`python3` …), `rm`, `dd`, `chmod`, `kill`, `curl`, `scp` and the like, also as `/bin/rm` or `python3.12`, get Allow for This Conversation only. Allow for This Conversation covers the same, until the app quits. An MCP tool's **Always allow** is its server's setting (`references/ai-mcp.md`).
