# Troubleshooting: Cloud Sync and Cloudflare

Local use never depends on sync: hosts, keys and sessions keep working when sync fails. Setup and upgrade steps are in `references/sync.md` and `references/sync-worker.md`. The messages are shown exactly as the app prints them in each language.

## Sync states

The sidebar's sync button and the Cloud Sync page show one of these. Changes made meanwhile stay on the device and upload later.

| en | zh-CN | ja |
|---|---|---|
| Offline | 离线 | オフライン |
| Will retry automatically | 将自动重试 | 自動的に再試行します |
| Signed out | 认证失效 | 認証切れ |
| Enter your master password again | 请重新输入主密码 | マスターパスワードを再入力してください |
| Sync error | 同步出错 | 同期エラー |
| Worker update required | 需要更新 Worker | Worker の更新が必要 |
| Hatoba update required | 需要更新 Hatoba | Hatoba の更新が必要 |
| Sync is off | 未开启云同步 | クラウド同期はオフ |
| Stored on this device only | 数据仅保存在本机 | このデバイスにのみ保存 |

| State | What it means | What to do |
|---|---|---|
| Offline | The Worker (or Cloudflare) can't be reached | Check the network and the Worker URL (**Endpoint** on the status page). It retries by itself; **Sync Now** retries at once |
| Signed out | The session expired or was revoked (another device pressed **Remove** on this one, or the master password changed) | Enter the master password under **Sign In Again** on the status page |
| Sync error | A sync round failed; the page shows the message | Read the message below; it retries automatically |
| Worker update required | The Worker is too old for this Hatoba | **Update Worker**; see `references/sync-worker.md` |
| Hatoba update required | The Worker is newer than this Hatoba supports | Update Hatoba: Settings → About → **Check for Updates** |
| Sync is off | No sync configured on this device | Set it up under **Cloud Sync** |

| en | zh-CN | ja |
|---|---|---|
| Sign In Again | 重新登录 | 再ログイン |
| Sync Now | 立即同步 | 今すぐ同期 |
| Endpoint | 端点 | エンドポイント |
| Update Worker | 更新 Worker | Worker を更新 |
| Check for Updates | 检查更新 | アップデートを確認 |

## Messages

| en | zh-CN | ja |
|---|---|---|
| Sync failed. | 同步失败。 | 同期に失敗しました。 |
| Can’t reach the sync service. Check your network. | 无法连接到同步服务，请检查网络。 | 同期サービスに接続できません。ネットワークを確認してください。 |
| Cloud Sync signed out. Enter your master password again. | 云同步认证失效，请重新输入主密码。 | クラウド同期の認証が切れました。マスターパスワードを再入力してください。 |
| The cloud already has a vault. On a new device, choose “Restore from Cloud”. | 云端已有保险库，请在新设备上选择“从云端恢复”。 | クラウドにはすでに保管庫があります。新しいデバイスでは「クラウドから復元」を選んでください。 |
| The cloud has no vault yet. Turn on Cloud Sync on a device that has your data first. | 云端还没有保险库。请先在已有数据的设备上开启云同步。 | クラウドにまだ保管庫がありません。先にデータのあるデバイスでクラウド同期をオンにしてください。 |
| Connection failed. Check the URL and your network. | 连接失败，请检查地址和网络。 | 接続に失敗しました。URL とネットワークを確認してください。 |
| Enter the full Worker address, e.g. https://hatoba-sync.example.workers.dev. | 请输入完整的 Worker 地址，例如 https://hatoba-sync.example.workers.dev。 | Worker の完全なアドレスを入力してください（例: https://hatoba-sync.example.workers.dev）。 |
| Incorrect master password. | 主密码不正确。 | マスターパスワードが正しくありません。 |
| Cancelled. | 已取消。 | キャンセルしました。 |

- "The cloud already has a vault…" appears when setting up a device whose local vault would have to merge with a cloud vault that already has data. This version cannot merge. Either restore on a fresh install with **Restore from Cloud**, or deploy under a different Worker name.
- "The cloud has no vault yet…" appears when restoring from a Worker or database that was never set up. Set sync up first on the device that has the data.
- "Incorrect master password." during setup or restore: enter the master password of the vault that was uploaded, not a new one. If it changed on another device, use the new one.
- For a Worker URL, use the full address including `https://`. A quick check is opening `<Worker URL>/v1/health` in a browser: it should return JSON with `"service": "hatoba-sync"`, a `version`, and `initialized`.

## Cloudflare and deployment errors

These come from **Deploy a Worker from Hatoba** and from direct D1 mode.

| en | zh-CN | ja |
|---|---|---|
| Cloudflare didn’t accept this API token. Check that it’s complete and not expired or disabled. An account-owned token also needs the account ID. | Cloudflare 不接受这个 API Token。请检查它是否完整、是否已过期或停用；账户所属的 Token 还需要填写 Account ID。 | Cloudflare がこの API トークンを受け付けませんでした。トークンが完全か、期限切れや無効になっていないかを確認してください。アカウント所有のトークンには Account ID も必要です。 |
| Cloudflare returned an error. Try again later. | Cloudflare 返回了错误，请稍后重试。 | Cloudflare がエラーを返しました。しばらくしてからもう一度お試しください。 |
| Choose a workers.dev subdomain for this account. | 请为这个账户选择一个 workers.dev 子域名。 | このアカウントの workers.dev サブドメインを選んでください。 |
| This workers.dev subdomain is taken. Choose another. | 这个 workers.dev 子域名已被占用，请换一个。 | この workers.dev サブドメインはすでに使われています。別の名前を選んでください。 |
| Another Worker already has this name, and it isn’t a Hatoba sync Worker. Choose another name. | 已有同名的 Worker，但它不是 Hatoba 的同步 Worker。请换一个名称。 | 同じ名前の Worker がありますが、Hatoba の同期 Worker ではありません。別の名前を選んでください。 |
| This account has no Hatoba Worker with this name, or it holds no vault. Check the account and the Worker name. | 这个账户中找不到这个名称的 Hatoba Worker，或者它没有保存保险库。请检查账户和 Worker 名称。 | このアカウントにこの名前の Hatoba Worker がないか、保管庫がありません。アカウントと Worker 名を確認してください。 |
| This Worker is newer than the one in this version of Hatoba, and Hatoba won’t replace it with an older one. | 这个 Worker 比这个版本的 Hatoba 内置的更新，Hatoba 不会用旧版本替换它。 | この Worker はこのバージョンの Hatoba に含まれるものより新しいため、Hatoba は古いバージョンで置き換えません。 |
| This build of Hatoba doesn’t include the sync Worker, so it can’t deploy it. Use another way to deploy. | 这个版本的 Hatoba 没有内置同步 Worker，无法一键部署。请改用其他部署方式。 | このビルドの Hatoba には同期 Worker が含まれていないため、デプロイできません。別の方法でデプロイしてください。 |
| This token can’t edit D1. Add “Account · D1 · Edit” to it in Cloudflare. | 这个 Token 没有 D1 编辑权限。请在 Cloudflare 中为它添加「Account · D1 · Edit」。 | このトークンには D1 の編集権限がありません。Cloudflare で「Account · D1 · Edit」を追加してください。 |
| Can’t reach Cloudflare. Check your network and try again. | 无法连接到 Cloudflare，请检查网络后重试。 | Cloudflare に接続できません。ネットワークを確認して、もう一度お試しください。 |

- Token rejected: the token is incomplete, expired, disabled, or an account-owned token without the **Account ID**. Create a new one with **Create a token in Cloudflare**; the link fills in both permissions.
- Missing permission: the message names the dashboard permission, "Account · Workers Scripts · Edit" or "Account · D1 · Edit" (direct D1 mode needs only D1). Edit the token in Cloudflare (**Edit token in Cloudflare**), add it, and retry.
- Cloudflare error (code N): Cloudflare returned an error; the code is shown. Wait and **Retry**; check Cloudflare's status page if it persists.
- The workers.dev subdomain: choose one when the account has none; if it is taken, choose another. A new subdomain can take a few minutes to answer; the page waits with **Check again**.
- Name taken: another Worker (not a Hatoba sync Worker) has the name, or the Worker already holds a vault. Hatoba never overwrites either. Choose another Worker name under **Worker and database names**, or restore the existing vault on a new device.
- A Worker is newer: the Worker already runs a newer version than this Hatoba carries; Hatoba does not downgrade it.
- This build has no Worker: a development build without the bundled Worker cannot deploy in-app. Use the Deploy to Cloudflare button or wrangler (**Connect a deployed Worker**).
- A failed step shows the error; **Retry** continues from it, and **Remove what Hatoba created** removes only what this deployment made.

| en | zh-CN | ja |
|---|---|---|
| Cloud Sync | 云同步 | クラウド同期 |
| Restore from Cloud | 从云端恢复 | クラウドから復元 |
| Deploy a Worker from Hatoba | 从 Hatoba 部署 Worker | Hatoba から Worker をデプロイ |
| Connect a deployed Worker | 连接已部署的 Worker | デプロイ済みの Worker に接続 |
| Account ID | Account ID | Account ID |
| workers.dev subdomain | workers.dev 子域名 | workers.dev サブドメイン |
| Check again | 再次检查 | もう一度確認 |
| Remove what Hatoba created | 移除 Hatoba 创建的内容 | Hatoba が作成したものを削除 |
| Check the token and account | 检查 Token 和账户 | トークンとアカウントを確認 |
| Look for an existing Worker and database | 查找已有的 Worker 和数据库 | 既存の Worker とデータベースを確認 |
| Create the database | 创建数据库 | データベースを作成 |
| Create the tables | 创建数据表 | テーブルを作成 |
| Upload the Worker | 上传 Worker | Worker をアップロード |
| Set the setup token | 设置 Setup Token | Setup Token を設定 |
| Turn on the workers.dev URL | 启用 workers.dev 地址 | workers.dev の URL を有効化 |
| Wait for the Worker to answer | 等待 Worker 响应 | Worker の応答を待機 |
| Create a token in Cloudflare | 在 Cloudflare 中创建 Token | Cloudflare でトークンを作成 |
| Edit token in Cloudflare | 在 Cloudflare 中编辑 Token | Cloudflare でトークンを編集 |
| Account ID (optional) | Account ID（可选） | Account ID（任意） |
| Worker and database names | Worker 和数据库名称 | Worker とデータベースの名前 |
| Retry | 重试 | 再試行 |

## Devices and conflicts

- A device that should not sync any more: **Remove** it on the status page (**Devices**). It then needs the master password to sync again; no data is deleted.
- "N conflicts were resolved automatically" is not an error. The newer edit wins; for SSH keys the loser is kept as a copy. Open **Review** to keep the result or restore the other version.
- Two devices that show different data: press **Sync Now** on both; a device that is offline or signed out has not uploaded yet.

## Still not solved

If the problem is not covered here and looks like a bug, use the feedback path in SKILL.md ("Project, feedback and newer information"): **Report a Problem** in Settings → About opens GitHub's bug report form (say whether sync uses a Worker or direct D1, and the Worker's version from `/v1/health`). Offer to draft the text, remind the user to leave out Worker URLs they consider private, tokens, keys, passwords and recovery codes, and let the user submit it.

| en | zh-CN | ja |
|---|---|---|
| Report a Problem | 反馈问题 | 問題を報告 |
| Review | 查看 | 確認 |
| Devices | 设备 | デバイス |
| Remove | 移除 | 削除 |
