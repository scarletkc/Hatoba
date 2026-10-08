# Troubleshooting: AI assistant, search, skills and MCP

Settings are in `references/ai-settings.md` and the panel in `references/ai-panel.md`. The messages are shown exactly as the app prints them in each language. Messages that contain a status code or the provider's own text are paraphrased.

## The provider returned an error

A failed request ends the turn with a banner in the panel ("The request failed", or "The provider returned an error (HTTP N)") and a **Retry** button. The provider's own message follows, untranslated. Typical statuses:

| Status | Usual cause | What to do |
|---|---|---|
| 401, 403 | The API key is wrong, expired or lacks access to the model; for the Anthropic protocol, the wrong **Auth Header** | Edit the provider: **Replace** the key; try the other **Auth Header** |
| 404 | The **Base URL** is wrong (a missing `/v1` for Chat Completions) or the model ID does not exist | Fix the base URL; check the model ID with **Fetch Models** |
| 400 | The provider rejected the request: the model does not support tool calls or a parameter, or the conversation is too long | Choose a model that supports tools; **Compact** the conversation; check **Max Output** |
| 413, "context length" | The conversation is over the model's context window | **Compact**, or start a **New Conversation**; set the right **Context** value for the model |
| 429 | Rate limit or quota used up | Wait, or switch model or provider |
| 5xx | The provider has a problem | **Retry** later |

| en | zh-CN | ja |
|---|---|---|
| The response hit the model’s output limit and was cut off. | 回复达到了模型的输出上限，后面的内容被截断了。 | 応答がモデルの出力上限に達したため、途中で切れました。 |
| The model declined to respond. | 模型拒绝了这次请求。 | モデルが応答を断りました。 |
| The request failed | 请求失败 | リクエストに失敗しました |
| The model returned nothing. | 模型没有返回内容。 | モデルから内容が返されませんでした。 |
| Add a model in Settings → AI first. | 请先在设置 → AI 中添加模型。 | 先に設定 → AI でモデルを追加してください。 |
| This conversation can’t be opened. | 无法打开这个对话。 | この会話を開けません。 |
| History can’t be loaded. | 无法读取历史对话。 | 履歴を読み込めません。 |
| The search failed. | 搜索失败。 | 検索に失敗しました。 |
| Stop | 停止 | 停止 |
| Retry | 重试 | 再試行 |
| Compact | 压缩 | 圧縮 |
| Auth Header | 认证头 | 認証ヘッダー |
| Replace | 替换 | 置き換え |
| Fetch Models | 获取模型列表 | モデル一覧を取得 |
| Context | 上下文窗口 | コンテキスト |
| Max Output | 输出上限 | 出力上限 |
| New Conversation | 新对话 | 新しい会話 |
| Base URL | 基础 URL | ベース URL |
| Models | 模型 | モデル |

- "The response hit the model's output limit…": raise **Max Output** for that model (for the Anthropic protocol it is `max_tokens`) or ask for a shorter answer.
- "The model returned nothing.": the model produced no text; try again or switch model.
- Models that do not support tool calls cannot work as the assistant: the provider answers with an error as soon as a request carries tools.

## Test Connection and provider settings

**Test Connection** in the provider form, and in Settings → AI → Web Search, tells the failure apart:

| en | zh-CN | ja |
|---|---|---|
| Test Connection | 测试连接 | 接続をテスト |
| The provider rejected the API key. Check the key, and for the Anthropic protocol the auth header. | 服务商拒绝了 API 密钥。请检查密钥，Anthropic 协议还要检查认证头。 | プロバイダーが API キーを拒否しました。キーを確認してください。Anthropic プロトコルでは認証ヘッダーも確認してください。 |
| Couldn’t reach the server. Check the base URL and your network. | 连不上服务器。请检查基础 URL 和网络。 | サーバーに接続できません。ベース URL とネットワークを確認してください。 |
| The base URL can’t be used. It must be HTTPS, and only local and private network addresses can use HTTP. | 基础 URL 不可用：必须使用 HTTPS，只有本机和内网地址可以用 HTTP。 | ベース URL は使えません。HTTPS が必要で、HTTP が使えるのはローカルとプライベートネットワークのアドレスだけです。 |
| The provider returned an error. | 服务商返回了错误。 | プロバイダーがエラーを返しました。 |
| Enter a full address that starts with https://. A local server can use http://. | 请输入以 https:// 开头的完整地址，本地服务器可以用 http://。 | https:// で始まる完全なアドレスを入力してください。ローカルサーバーは http:// も使えます。 |
| The base URL must use HTTPS. Only local and private network addresses can use HTTP. | 基础 URL 必须使用 HTTPS，只有本机和内网地址可以用 HTTP。 | ベース URL には HTTPS が必要です。HTTP が使えるのは、ローカルとプライベートネットワークのアドレスだけです。 |
| Enter an API key. | 请输入 API 密钥。 | API キーを入力してください。 |
| Couldn’t load the model list. You can still type model IDs. | 无法获取模型列表。你仍然可以手动输入模型 ID。 | モデル一覧を取得できませんでした。モデル ID は手動で入力できます。 |
| Enter a number of tokens, such as 200000 or 200K. | 请输入 token 数，例如 200000 或 200K。 | トークン数を入力してください。例: 200000 または 200K |
| A model ID can’t be empty. | 模型 ID 不能为空。 | モデル ID は空にできません。 |

- "The provider doesn’t know the model …": the model ID has a typo or the provider lacks it. Fix it in **Models**.
- "Couldn’t reach the server": a wrong **Base URL**, no network, or a local server (Ollama, LM Studio) that is not running. For a local server `http://localhost:…` is allowed; any other plain-HTTP address is refused.
- A local server may need the model name exactly as it lists it, and a context window large enough for the conversation.

## Web search

Without a search provider the assistant has no web search tool. In Settings → AI → **Web Search**:

| en | zh-CN | ja |
|---|---|---|
| The search service rejected the API key. | 搜索服务拒绝了 API 密钥。 | 検索サービスが API キーを拒否しました。 |
| Couldn’t reach the search service. Check the address and your network. | 连不上搜索服务。请检查地址和网络。 | 検索サービスに接続できません。アドレスとネットワークを確認してください。 |
| The instance URL isn’t valid. Check it and try again. | 实例地址无效，请检查后重试。 | インスタンスの URL が正しくありません。確認してもう一度お試しください。 |
| The search service returned an error. | 搜索服务返回了错误。 | 検索サービスがエラーを返しました。 |
| Enter the instance URL. | 请输入实例地址。 | インスタンスの URL を入力してください。 |
| Web Search | 网页搜索 | Web 検索 |
| None | 无 | なし |
| SearXNG | SearXNG | SearXNG |

- For **SearXNG** the instance must allow the JSON output format (`search.formats` includes `json` in its settings).
- "Fetch page" refuses addresses on private, loopback or link-local networks, pages that are not text, HTML, JSON or XML, and needs approval in manual mode.

## Skills

| en | zh-CN | ja |
|---|---|---|
| Use only lowercase letters, digits, and hyphens, up to 64 characters. | 名称只能用小写字母、数字和连字符，最多 64 个字符。 | 小文字の英字、数字、ハイフンだけを使い、64 文字以内にしてください。 |
| Another skill already has this name. | 已有同名的技能。 | 同じ名前のスキルがすでにあります。 |
| Enter a description. | 请输入描述。 | 説明を入力してください。 |
| SKILL.md is over 32 KB. Move detail into other files. | SKILL.md 超过了 32 KB，请把细节放进其他文件。 | SKILL.md が 32 KB を超えています。詳細は他のファイルに移してください。 |
| A skill can hold at most 5 MB in total. | 一个技能的总大小不能超过 5 MB。 | 1 つのスキルの合計サイズは 5 MB までです。 |
| There is no SKILL.md at the top of the folder or .zip. | 文件夹或 .zip 的顶层没有 SKILL.md。 | フォルダーまたは .zip の最上位に SKILL.md がありません。 |
| Couldn’t read this folder or .zip. | 无法读取这个文件夹或 .zip。 | このフォルダーまたは .zip を読み込めませんでした。 |
| This skill can’t be imported | 无法导入这个技能 | このスキルはインポートできません |
| Skipped Files | 已跳过的文件 | スキップされたファイル |

- A skill name is 1 to 64 lowercase letters, digits and hyphens. The description is at most 1,024 characters; `SKILL.md` and each other file at most 32 KB; at most 200 files and 5 MB per skill; only UTF-8 text files.
- Import needs a `SKILL.md` at the top of the folder or `.zip`, with a valid frontmatter (`name` and `description`). The import dialog lists every problem. If the name exists, choose **Replace it** or **Import under a new name**.
- If the assistant does not use a skill, check that its switch is on and that the description says when to use it.

## MCP servers

| en | zh-CN | ja |
|---|---|---|
| It failed to start or to answer. | 启动失败，或没有回应。 | 起動できなかったか、応答がありませんでした。 |
| Failed | 失败 | 失敗 |
| Last lines of stderr | stderr 的最后几行 | stderr の最後の行 |
| Enter the command. | 请输入命令。 | コマンドを入力してください。 |
| The URL must use HTTPS. Only local and private network addresses can use HTTP. | URL 必须使用 HTTPS，只有本机和内网地址可以用 HTTP。 | URL には HTTPS が必要です。HTTP が使えるのは、ローカルとプライベートネットワークのアドレスだけです。 |
| This looks like a whole command line. Put only the program here, and each argument on its own row. | 这像是一整行命令。请在这里只填程序，每个参数单独占一行。 | コマンドライン全体のようです。ここにはプログラムだけを入力し、引数は 1 行ずつ分けてください。 |
| Split Into Arguments | 拆成参数 | 引数に分割 |
| A quote isn’t closed. Check the command. | 有引号没有闭合，请检查。 | 閉じられていない引用符があります。コマンドを確認してください。 |
| Nothing in this JSON can be imported. | 这段 JSON 里没有可以导入的服务器。 | この JSON にはインポートできるものがありません。 |
| Start | 启动 | 起動 |
| Restart | 重新启动 | 再起動 |

- A server that fails to start shows **Failed** with the error and the last lines of stderr, in Settings → AI → **MCP Servers** (click its row) and in the **MCP tools** menu of the panel. Its tool calls return errors meanwhile. Read the stderr, fix the command or the environment, then **Restart**.
- Command servers are started like a shell would: `npx` finds `npx.cmd` on Windows. Put only the program in **Command** and each argument on its own row; if the whole command line was pasted, use **Split Into Arguments**. A missing program or a wrong `PATH` is the usual cause of a failed start.
- An HTTP server needs HTTPS (plain HTTP only for local and private addresses). Servers that need OAuth sign-in are not supported; use a token in **Headers** if the service allows it.
- Servers start when a conversation first needs their tools and stop on quit or lock; after unlocking they start again when needed. A server switched off for this device, or for the conversation in the **MCP tools** menu, offers no tools.
- A server added on another device arrives off if it is a Command server; switch it on here after checking the command.

## Still not solved

If the problem is not covered by the troubleshooting files and looks like a bug, use the feedback path in SKILL.md ("Project, feedback and newer information"): **Report a Problem** in Settings → About opens GitHub's bug report form. Offer to draft the text (provider type and protocol, model, the HTTP status and the error text, the version), and remind the user to remove API keys, tokens, private URLs and hostnames before sending. The user submits it.

| en | zh-CN | ja |
|---|---|---|
| Report a Problem | 反馈问题 | 問題を報告 |
| Replace it | 替换它 | 置き換える |
| Import under a new name | 用新名称导入 | 新しい名前でインポート |
| Command | 命令 | コマンド |
| Headers | 请求头 | ヘッダー |
| MCP Servers | MCP 服务器 | MCP サーバー |
| AI | AI | AI |
| MCP tools | MCP 工具 | MCP ツール |
