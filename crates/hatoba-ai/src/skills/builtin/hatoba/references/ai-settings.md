# Settings → AI

Open Settings (the gear in the sidebar, or Ctrl+, / ⌘,) and choose the **AI** tab. It has these sections in order: Providers, Default Model, Web Search, Custom Instructions (`references/ai-instructions.md`), Skills, MCP Servers and This Device. Using the assistant is covered in `references/ai-panel.md`.

| en | zh-CN | ja |
|---|---|---|
| Settings | 设置 | 設定 |
| AI | AI | AI |
| Providers | 服务商 | プロバイダー |
| Default Model | 默认模型 | 既定のモデル |
| Web Search | 网页搜索 | Web 検索 |
| Custom Instructions | 自定义指令 | カスタム指示 |
| Skills | 技能 | スキル |
| MCP Servers | MCP 服务器 | MCP サーバー |
| This Device | 此设备 | このデバイス |

## Providers

The assistant calls the providers added here directly from the device, with the user's own API keys. Hatoba runs no AI service, and nothing is sent until a provider is added and a message is sent. The provider must support tool calls. Local servers such as Ollama and LM Studio work.

| en | zh-CN | ja |
|---|---|---|
| Add Provider | 添加服务商 | プロバイダーを追加 |
| Name | 名称 | 名前 |
| Protocol | 协议 | プロトコル |
| OpenAI Chat Completions | OpenAI Chat Completions | OpenAI Chat Completions |
| Anthropic Messages | Anthropic Messages | Anthropic Messages |
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
- **Protocol**: **OpenAI Chat Completions** (OpenAI, Gemini through its OpenAI-compatible endpoint, local servers, most other services) or **Anthropic Messages** (Claude and vendors with an Anthropic-compatible endpoint). The form picks Anthropic for `api.anthropic.com` and for base URLs ending in `/anthropic`.
- **Base URL**: for Chat Completions include the API version, such as `https://api.openai.com/v1`. For Anthropic use the address the vendor documents for Anthropic SDKs, such as `https://api.anthropic.com`. HTTPS is required; plain HTTP is allowed only for local and private-network addresses.
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

- **Model for new conversations** is the model a new conversation starts with. It needs at least one provider with a model, and it syncs. Each conversation then keeps the model it used last.
- **Thinking level for new conversations** is the level a new conversation starts at: Default, Low, Medium, High, Extra High or Max (see `references/ai-models.md`). It syncs. A model that does not offer that level uses the highest lower level it has. Each conversation then keeps the level it used last. What is sent: for OpenAI Chat Completions a `reasoning_effort` of low, medium or high (xhigh or max only for a model whose levels include them); for Anthropic Messages an `output_config` effort, with adaptive thinking switched on for models that support it. Default sends none of these.
- **Search provider** is the backend of the assistant's web search. Choose **Brave Search API** or **Tavily** (enter the **API Key**) or **SearXNG** (enter the **Instance URL** of your own instance, which must have the JSON format enabled), then **Test Connection** and **Save**. **None** (the default) means the assistant has no web search tool. Search terms go to the chosen service. **Remove Configuration** deletes the saved settings and key.

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
- **Import** has **Import Folder…** and **Import .zip…** for a folder or archive with a `SKILL.md` at its top. **Import Skill** lists every file to be saved before anything is stored, and saves only what it showed: if the files changed on disk in between, the import asks to preview them again. Files that are not UTF-8 text are skipped and listed, and a file over 32 KB or too large to sync once encoded is refused. If the name is taken, choose **Replace it** or **Import under a new name**.
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

MCP (Model Context Protocol) servers add tools to the assistant. Hatoba uses tools only (no prompts or resources). Two kinds: **Command** (stdio: a program started on this device with the user's privileges, which Hatoba talks to over standard input and output) and **HTTP** (a Streamable HTTP URL). A stdio server runs arbitrary code, so add only servers you trust. Tool descriptions reach the model as instructions.

| en | zh-CN | ja |
|---|---|---|
| Add Server | 添加服务器 | サーバーを追加 |
| Export | 导出 | エクスポート |
| Add MCP Server | 添加 MCP 服务器 | MCP サーバーを追加 |
| Type | 类型 | 種類 |
| Command | 命令 | コマンド |
| HTTP | HTTP | HTTP |
| Arguments | 参数 | 引数 |
| Split Into Arguments | 拆成参数 | 引数に分割 |
| Environment | 环境变量 | 環境変数 |
| URL | URL | URL |
| Headers | 请求头 | ヘッダー |
| Always ask | 始终询问 | 常に確認 |
| Save this command? | 保存这条命令？ | このコマンドを保存しますか？ |
| Save Server | 保存服务器 | サーバーを保存 |
| Import MCP Servers | 导入 MCP 服务器 | MCP サーバーをインポート |
| Preview | 预览 | プレビュー |

- **Add Server**: give it a **Name** and choose the **Type**. For **Command**: the program (looked up on `PATH` like a shell does, so `npx` finds `npx.cmd` on Windows), the **Arguments** (one per row; if a whole command line is pasted into the command, **Split Into Arguments** separates them) and **Environment** variables. For **HTTP**: the **URL** (HTTPS required; HTTP only for local and private addresses) and **Headers** such as `Authorization`. Environment and header values are stored encrypted, can't be viewed after saving, and only the program or the server receives them. Saving a Command server first shows **Save this command?** with the full command line.
- **Always ask** makes this server's tools ask before every call, even in Bypass mode. It syncs with the server.
- **Import** reads pasted JSON or a JSON file in the `mcpServers` format of Claude Desktop, Claude Code and Cursor (or VS Code's `servers` key), shows a **Preview**, and imports; values move into the vault. **Export** writes the same format with placeholders instead of the secret values.
- The server list syncs. Whether a server is on, and any **Always allow**, stay on each device. A server added on another device arrives on if it is HTTP and off if it is a Command. A Command server whose command, arguments or environment variable names change on another device turns off here and loses its **Always allow** until it is switched on here again.
- A Command server starts when a conversation first needs its tools and stops when the app quits or the vault locks. Click a row to see its state, start, restart or stop it, read the error with the last lines of stderr, and see its tools with their hints (Read-only, Destructive, Idempotent, Open-world). Hints never change whether a call asks.

| en | zh-CN | ja |
|---|---|---|
| Stopped | 已停止 | 停止中 |
| Starting… | 正在启动… | 起動中… |
| Running | 运行中 | 実行中 |
| Failed | 失败 | 失敗 |
| Off on this device | 此设备上已关闭 | このデバイスではオフ |
| Start | 启动 | 起動 |
| Restart | 重新启动 | 再起動 |
| Stop | 停止 | 停止 |
| Always allow all tools | 始终允许所有工具 | すべてのツールを常に許可 |
| Always allow | 始终允许 | 常に許可 |
| Tools | 工具 | ツール |
| Last lines of stderr | stderr 的最后几行 | stderr の最後の行 |

- In manual mode every MCP tool asks for approval. **Always allow all tools** (per server) or **Always allow** (per tool) lets calls run without asking on this device. OAuth sign-in for MCP servers is not supported.

## This Device

These two settings stay on this device and are not synced.

| en | zh-CN | ja |
|---|---|---|
| Default Permission Mode | 默认权限模式 | 既定の権限モード |
| Manual approval | 手动批准 | 手動承認 |
| Bypass | 绕过批准 | 承認をバイパス |
| Tool Call Limit | 工具调用上限 | ツール呼び出しの上限 |
| Turn On Bypass | 开启绕过模式 | バイパスをオンにする |

- **Default Permission Mode**: **Manual approval** (default) or **Bypass**. New conversations start in it. Choosing Bypass the first time asks for confirmation.
- **Tool Call Limit**: a turn pauses after this many tool calls and goes on only when you press Continue. From 1 to 200, default 25.
