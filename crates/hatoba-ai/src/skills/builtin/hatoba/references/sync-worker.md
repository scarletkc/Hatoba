# Sync Worker: deploy, connect and upgrade

The sync Worker is a small Cloudflare Worker with a D1 database. The app talks only to the Worker, and both hold ciphertext only. The Cloudflare free plan usually covers personal use. The wizard is on the Cloud Sync page (`references/sync.md`). Step 2 (**Connect**) depends on the method chosen in step 1.

## Method 1: Deploy a Worker from Hatoba

Hatoba creates the Worker and the D1 database in the user's Cloudflare account through the Cloudflare API, with an API token it uses only during the deployment. The token stays in memory, is never saved, and is dropped when the wizard is left or the vault locks.

| en | zh-CN | ja |
|---|---|---|
| Connect | 连接 | 接続 |
| Deploy with an API Token | 用 API Token 部署 | API トークンでデプロイ |
| The token needs two permissions | Token 需要两项权限 | トークンには 2 つの権限が必要です |
| Create a token in Cloudflare | 在 Cloudflare 中创建 Token | Cloudflare でトークンを作成 |
| API Token | API Token | API トークン |
| Account ID (optional) | Account ID（可选） | Account ID（任意） |
| Review the Deployment | 确认部署内容 | デプロイ内容の確認 |
| Cloudflare account | Cloudflare 账户 | Cloudflare アカウント |
| workers.dev subdomain | workers.dev 子域名 | workers.dev サブドメイン |
| Worker and database names | Worker 和数据库名称 | Worker とデータベースの名前 |
| Worker name | Worker 名称 | Worker 名 |
| Database name | 数据库名称 | データベース名 |
| Deploy | 部署 | デプロイ |
| Continue | 继续 | 続ける |

1. **Deploy with an API Token**: the token needs two permissions, "Account · Workers Scripts · Edit" and "Account · D1 · Edit". **Create a token in Cloudflare** opens the Cloudflare token form with both filled in. Limit Account Resources to one account and set a short expiry, or delete the token after deploying. Paste the token into **API Token**. **Account ID (optional)** is needed only for an account-owned token; for a normal token Hatoba finds the account itself (find the ID under Workers & Pages → Account Details). Then **Continue**.
2. **Review the Deployment**: choose the **Cloudflare account** if the token reaches several. If the account has no workers.dev subdomain yet, choose one under **workers.dev subdomain** (it becomes part of every Worker URL in the account). **Worker and database names** (expandable) default to `hatoba-sync` and `hatoba`: lowercase letters, digits and hyphens for the Worker (up to 63 characters), plus underscores for the database (up to 60). The page lists what will be created or reused. Nothing changes in the account until **Deploy**.
3. **Deploy** runs the steps with one progress row each:

| en | zh-CN | ja |
|---|---|---|
| Check the token and account | 检查 Token 和账户 | トークンとアカウントを確認 |
| Look for an existing Worker and database | 查找已有的 Worker 和数据库 | 既存の Worker とデータベースを確認 |
| Create the database | 创建数据库 | データベースを作成 |
| Create the tables | 创建数据表 | テーブルを作成 |
| Upload the Worker | 上传 Worker | Worker をアップロード |
| Set the setup token | 设置 Setup Token | Setup Token を設定 |
| Turn on the workers.dev URL | 启用 workers.dev 地址 | workers.dev の URL を有効化 |
| Wait for the Worker to answer | 等待 Worker 响应 | Worker の応答を待機 |
| not needed | 无需操作 | 不要 |
| Sync Worker Deployed | 同步 Worker 已部署 | 同期 Worker をデプロイしました |
| Deployment Stopped | 部署没有完成 | デプロイが完了しませんでした |

4. When the Worker is live (**Sync Worker Deployed**), **Continue** goes to the master password step (`references/sync.md`). Hatoba sets up the cloud vault itself and then deletes the Worker's setup token secret.

Safety rules: Hatoba never overwrites a Worker that is not a Hatoba sync Worker, nor a Worker that already holds a vault, nor a database that holds other data (it picks a free name such as `hatoba-2`). Existing empty Hatoba pieces from an earlier attempt are reused.

If a step fails, the page shows the error:

| en | zh-CN | ja |
|---|---|---|
| Retry | 重试 | 再試行 |
| Remove what Hatoba created | 移除 Hatoba 创建的内容 | Hatoba が作成したものを削除 |
| Check again | 再次检查 | もう一度確認 |
| Back | 返回 | 戻る |

- **Retry** continues from the step that failed. **Remove what Hatoba created** deletes only the Worker and database this deployment created; nothing that existed before is touched.
- A new workers.dev subdomain can take a few minutes before the Worker answers. The page then waits with **Check again**.
- Error texts for tokens, permissions and names are in `references/troubleshooting-sync.md`.

## Method 2: Connect a deployed Worker

For a Worker the user deployed with the Deploy to Cloudflare button or with wrangler. The step is titled **Deploy the Sync Worker**.

| en | zh-CN | ja |
|---|---|---|
| Deploy the Sync Worker | 部署同步 Worker | 同期 Worker をデプロイ |
| One-click deploy | 一键部署 | ワンクリックデプロイ |
| Deploy to Cloudflare | 部署到 Cloudflare | Cloudflare にデプロイ |
| Deploy manually with wrangler | 使用 wrangler 手动部署 | wrangler で手動デプロイ |
| Full deployment guide | 完整部署说明 | 詳しいデプロイ手順 |
| Worker URL | Worker URL | Worker URL |
| Setup Token | Setup Token | Setup Token |
| Test Connection | 测试连接 | 接続をテスト |
| Connected | 已连接 | 接続しました |

- **Deploy to Cloudflare** (**One-click deploy**) opens Cloudflare in the browser, which creates the Worker and the D1 database. Or open **Deploy manually with wrangler** and run its commands in order in the repository's `workers/sync` folder (`npm install`, `npx wrangler d1 create hatoba`, `npx wrangler d1 migrations apply hatoba --remote`, `npx wrangler secret put SETUP_TOKEN`, `npx wrangler deploy`). **Full deployment guide** opens the guide.
- Enter the **Worker URL** (for example `https://hatoba-sync.<name>.workers.dev`) and the **Setup Token** you set with `wrangler secret put SETUP_TOKEN`; it is used only for the first setup. **Test Connection** shows **Connected** with the Worker version and latency.
- **Continue** is enabled only when the Worker has no vault yet. If it already has one, the message is "The cloud already has a vault. On a new device, choose “Restore from Cloud”."

## Method 3: Use an API Token (direct D1)

Titled **Enter an API Token**. Nothing is deployed; the app calls the D1 HTTP API directly.

| en | zh-CN | ja |
|---|---|---|
| Enter an API Token | 填写 API Token | API トークンを入力 |
| Account ID | Account ID | Account ID |
| Verify Token | 验证 Token | トークンを確認 |
| Database | 数据库 | データベース |
| Choose a database | 选择数据库 | データベースを選択 |
| Edit token in Cloudflare | 在 Cloudflare 中编辑 Token | Cloudflare でトークンを編集 |

- Enter the **Account ID** and an **API Token** that has only "Account · D1 · Edit", then **Verify Token**. Pick the **Database** (it must already exist; create one first with `wrangler d1 create hatoba`). **Edit token in Cloudflare** appears when the token lacks the D1 permission.
- Such a token can usually edit every D1 database in the account. It is kept only in this device's credential store. Prefer a Worker unless that is acceptable.

## Upgrading the Worker

The app compares the Worker's version with the one it carries whenever the Cloud Sync page opens and at sync time. The status page then shows one of:

| en | zh-CN | ja |
|---|---|---|
| Update Worker | 更新 Worker | Worker を更新 |
| Dismiss | 忽略 | 閉じる |
| Check for updates | 检查更新 | アップデートを確認 |
| Update the Sync Worker | 更新同步 Worker | 同期 Worker の更新 |
| Review the Update | 确认更新内容 | 更新内容の確認 |
| Update | 更新 | 更新 |
| Updating the Sync Worker | 正在更新同步 Worker | 同期 Worker を更新しています |
| Sync Worker Updated | 同步 Worker 已更新 | 同期 Worker を更新しました |
| Update Stopped | 更新没有完成 | 更新が完了しませんでした |
| Check the Worker and its database | 检查 Worker 和它的数据库 | Worker とそのデータベースを確認 |
| Update the tables | 更新数据表 | テーブルを更新 |
| Upgrade steps | 升级步骤 | アップグレード手順 |
| Worker update required | 需要更新 Worker | Worker の更新が必要 |
| Hatoba update required | 需要更新 Hatoba | Hatoba の更新が必要 |

- "A sync Worker update is available": sync keeps working. **Update Worker** starts the upgrade; **Dismiss** hides the notice until the next Worker version.
- **Worker update required**: this version of Hatoba cannot sync with the old Worker. Sync pauses (local changes stay on the device and upload after the update) until the Worker is updated.
- **Hatoba update required**: the Worker is newer than this app supports. Update Hatoba (**Check for updates** opens Settings → About). Hatoba never replaces a newer Worker with an older one.

**Update Worker** opens **Update the Sync Worker**. Enter an API token with the same two permissions, an Account ID if asked, and the Worker name if the Worker uses a custom domain (otherwise it is the first part of its workers.dev URL). **Continue**, check **Review the Update** (it shows the table changes and the version change), then **Update**. Progress rows follow; **Retry** continues after a failure, and the Worker keeps working meanwhile. The cloud vault and its data are not touched.

If the Worker was deployed with the Deploy to Cloudflare button, Cloudflare redeploys it from the user's repository on its next push, which undoes an in-app update. In that case update `workers/sync` in the repository by the **Upgrade steps** in the deployment guide.
