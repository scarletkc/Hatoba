# Japanese labels: Cloud Sync and the sync Worker

The labels that these files name in English, as the interface shows them in Japanese (ja). Quote them exactly; the same English label can read differently in another file or section. A word in braces, such as {host}, stands for the value the interface fills in.

## `references/sync.md`

| en | ja |
|---|---|
| Cloud Sync | クラウド同期 |
| Vault | 保管庫 |
| Sync is off | クラウド同期はオフ |
| Stored on this device only | このデバイスにのみ保存 |
| Synced | 同期済み |
| Syncing… | 同期中… |
| Offline | オフライン |
| Will retry automatically | 自動的に再試行します |
| Signed out | 認証切れ |
| Enter your master password again | マスターパスワードを再入力してください |
| Sync error | 同期エラー |
| Worker update required | Worker の更新が必要 |
| Hatoba update required | Hatoba の更新が必要 |
| Resolved automatically, review pending | 自動的に解決済み・確認待ち |

### Set up on the first device

| en | ja |
|---|---|
| Method | 接続方法 |
| Connect | 接続 |
| Password | パスワード |
| Deploy a Worker from Hatoba | Hatoba から Worker をデプロイ |
| Recommended | 推奨 |
| Connect a deployed Worker | デプロイ済みの Worker に接続 |
| Use an API Token | API トークンを直接入力 |
| How is my data encrypted? | データはどう暗号化される？ |
| Continue | 続ける |
| Cancel | キャンセル |
| Back | 戻る |
| Enter Your Master Password | マスターパスワードを入力 |
| Master Password | マスターパスワード |
| Finish Setup | 設定を完了 |
| Cloud Sync is on | クラウド同期をオンにしました |
| Uploading {n} items… | {n} 件をアップロード中… |

### Join a cloud vault

| en | ja |
|---|---|
| Connected · the cloud already holds a vault | 接続しました · クラウドに保管庫があります |
| Enter the Cloud Vault’s Master Password | クラウドの保管庫のマスターパスワードを入力 |
| Merge and Turn On Sync | 統合して同期をオン |
| Merging… | 統合しています… |

### Add another device

| en | ja |
|---|---|
| Restore from Cloud | クラウドから復元 |
| Method | 接続方式 |
| Sync Worker | 同期 Worker |
| Direct D1 | D1 直接接続 |
| Worker URL | Worker URL |
| Test Connection | 接続テスト |
| Restore Vault | 保管庫を復元 |

### The status page

| en | ja |
|---|---|
| Sync Now | 今すぐ同期 |
| Connection | 接続 |
| Endpoint | エンドポイント |
| Database | データベース |
| Sync automatically | 自動同期 |
| Devices | デバイス |
| This PC | このPC |
| Not seen in 14+ days | 14 日以上同期されていません |
| Remove | 削除 |
| Sign In Again | 再ログイン |
| Remove Device | デバイスを削除 |

### Conflicts

| en | ja |
|---|---|
| Review | 確認 |
| Keep Current Result | 現在の結果を維持 |
| Restore Other Version | もう一方を復元 |
| Cloud | クラウド |
| No conflicts to review. | 確認が必要な競合はありません。 |
| Kept both versions; one is marked “(conflict copy)” | 両方のバージョンを残し、片方に「（競合コピー）」を付けました |

### Security actions and disconnecting

| en | ja |
|---|---|
| Always ask | 常に確認 |
| Generate New Recovery Code | 新しいリカバリーコードを作成 |
| Change Master Password | マスターパスワードを変更 |
| Disconnect… | クラウド同期を解除… |
| Disconnect Cloud Sync? | クラウド同期を解除しますか？ |
| Disconnect | 解除 |

## `references/sync-worker.md`

### Method 1: Deploy a Worker from Hatoba

| en | ja |
|---|---|
| Connect | 接続 |
| Deploy with an API Token | API トークンでデプロイ |
| The token needs two permissions | トークンには 2 つの権限が必要です |
| Create a token in Cloudflare | Cloudflare でトークンを作成 |
| API Token | API トークン |
| Account ID (optional) | Account ID（任意） |
| Review the Deployment | デプロイ内容の確認 |
| Cloudflare account | Cloudflare アカウント |
| workers.dev subdomain | workers.dev サブドメイン |
| Worker and database names | Worker とデータベースの名前 |
| Worker name | Worker 名 |
| Database name | データベース名 |
| Deploy | デプロイ |
| Continue | 続ける |
| Check the token and account | トークンとアカウントを確認 |
| Look for an existing Worker and database | 既存の Worker とデータベースを確認 |
| Create the database | データベースを作成 |
| Create the tables | テーブルを作成 |
| Upload the Worker | Worker をアップロード |
| Set the setup token | Setup Token を設定 |
| Turn on the workers.dev URL | workers.dev の URL を有効化 |
| Wait for the Worker to answer | Worker の応答を待機 |
| not needed | 不要 |
| Sync Worker Deployed | 同期 Worker をデプロイしました |
| Deployment Stopped | デプロイが完了しませんでした |
| Retry | 再試行 |
| Remove what Hatoba created | Hatoba が作成したものを削除 |
| Check again | もう一度確認 |
| Back | 戻る |

### Method 2: Connect a deployed Worker

| en | ja |
|---|---|
| Deploy the Sync Worker | 同期 Worker をデプロイ |
| One-click deploy | ワンクリックデプロイ |
| Deploy to Cloudflare | Cloudflare にデプロイ |
| Deploy manually with wrangler | wrangler で手動デプロイ |
| Full deployment guide | 詳しいデプロイ手順 |
| Worker URL | Worker URL |
| Setup Token | Setup Token |
| Test Connection | 接続をテスト |
| Connected | 接続しました |

### Method 3: Use an API Token (direct D1)

| en | ja |
|---|---|
| Enter an API Token | API トークンを入力 |
| Account ID | Account ID |
| Verify Token | トークンを確認 |
| Database | データベース |
| Choose a database | データベースを選択 |
| Edit token in Cloudflare | Cloudflare でトークンを編集 |

### Upgrading the Worker

| en | ja |
|---|---|
| Update Worker | Worker を更新 |
| Dismiss | 閉じる |
| Check for updates | アップデートを確認 |
| Update the Sync Worker | 同期 Worker の更新 |
| Review the Update | 更新内容の確認 |
| Update | 更新 |
| Updating the Sync Worker | 同期 Worker を更新しています |
| Sync Worker Updated | 同期 Worker を更新しました |
| Update Stopped | 更新が完了しませんでした |
| Check the Worker and its database | Worker とそのデータベースを確認 |
| Update the tables | テーブルを更新 |
| Upgrade steps | アップグレード手順 |
| Worker update required | Worker の更新が必要 |
| Hatoba update required | Hatoba の更新が必要 |

## `references/troubleshooting-sync.md`

### Sync states

| en | ja |
|---|---|
| Offline | オフライン |
| Will retry automatically | 自動的に再試行します |
| Signed out | 認証切れ |
| Enter your master password again | マスターパスワードを再入力してください |
| Sync error | 同期エラー |
| Worker update required | Worker の更新が必要 |
| Hatoba update required | Hatoba の更新が必要 |
| Sync is off | クラウド同期はオフ |
| Stored on this device only | このデバイスにのみ保存 |
| Sign In Again | 再ログイン |
| Sync Now | 今すぐ同期 |
| Endpoint | エンドポイント |
| Update Worker | Worker を更新 |
| Check for Updates | アップデートを確認 |

### Messages

| en | ja |
|---|---|
| Sync failed. | 同期に失敗しました。 |
| Can’t reach the sync service. Check your network. | 同期サービスに接続できません。ネットワークを確認してください。 |
| Cloud Sync signed out. Enter your master password again. | クラウド同期の認証が切れました。マスターパスワードを再入力してください。 |
| The cloud already holds a vault. Go back and connect again to join it. | クラウドにはすでに保管庫があります。前の手順に戻ってもう一度接続すると、その保管庫に参加できます。 |
| The cloud has no vault yet. Turn on Cloud Sync on a device that has your data first. | クラウドにまだ保管庫がありません。先にデータのあるデバイスでクラウド同期をオンにしてください。 |
| Another Cloud Sync setup is still running. Try again when it finishes. | 別のクラウド同期の設定がまだ進行中です。完了してからもう一度お試しください。 |
| Cloud Sync is already on for this device. Disconnect it before joining another cloud vault. | このデバイスではすでにクラウド同期がオンになっています。別のクラウドの保管庫に参加するには、先にクラウド同期を解除してください。 |
| Connection failed. Check the URL and your network. | 接続に失敗しました。URL とネットワークを確認してください。 |
| Enter the full Worker address, e.g. https://hatoba-sync.example.workers.dev. | Worker の完全なアドレスを入力してください（例: https://hatoba-sync.example.workers.dev）。 |
| Incorrect master password. | マスターパスワードが正しくありません。 |
| Cancelled. | キャンセルしました。 |

### An item too large to sync

| en | ja |
|---|---|
| Your settings are too large to sync. Shorten your custom instructions. | 設定のデータが大きすぎて同期できません。カスタム指示を短くしてください。 |
| This host is too large to sync. Shorten its notes or environment variables. | このホストはデータが大きすぎて同期できません。メモや環境変数を短くしてください。 |
| Edit Host | ホストを編集 |
| Open Settings → AI | 設定 → AI を開く |
| Custom Instructions | カスタム指示 |
| Host “{name}” is too large to sync. Shorten its notes, AI notes, or environment variables. | ホスト「{name}」はデータが大きすぎて同期できません。メモや AI 向けメモ、環境変数を短くしてください。 |

### Cloudflare and deployment errors

| en | ja |
|---|---|
| Cloudflare didn’t accept this API token. Check that it’s complete and not expired or disabled. An account-owned token also needs the account ID. | Cloudflare がこの API トークンを受け付けませんでした。トークンが完全か、期限切れや無効になっていないかを確認してください。アカウント所有のトークンには Account ID も必要です。 |
| Cloudflare returned an error. Try again later. | Cloudflare がエラーを返しました。しばらくしてからもう一度お試しください。 |
| Choose a workers.dev subdomain for this account. | このアカウントの workers.dev サブドメインを選んでください。 |
| This workers.dev subdomain is taken. Choose another. | この workers.dev サブドメインはすでに使われています。別の名前を選んでください。 |
| Another Worker already has this name, and it isn’t a Hatoba sync Worker. Choose another name. | 同じ名前の Worker がありますが、Hatoba の同期 Worker ではありません。別の名前を選んでください。 |
| This account has no Hatoba Worker with this name, or it holds no vault. Check the account and the Worker name. | このアカウントにこの名前の Hatoba Worker がないか、保管庫がありません。アカウントと Worker 名を確認してください。 |
| This Worker is newer than the one in this version of Hatoba, and Hatoba won’t replace it with an older one. | この Worker はこのバージョンの Hatoba に含まれるものより新しいため、Hatoba は古いバージョンで置き換えません。 |
| This build of Hatoba doesn’t include the sync Worker, so it can’t deploy it. Use another way to deploy. | このビルドの Hatoba には同期 Worker が含まれていないため、デプロイできません。別の方法でデプロイしてください。 |
| This token can’t edit D1. Add “Account · D1 · Edit” to it in Cloudflare. | このトークンには D1 の編集権限がありません。Cloudflare で「Account · D1 · Edit」を追加してください。 |
| Can’t reach Cloudflare. Check your network and try again. | Cloudflare に接続できません。ネットワークを確認して、もう一度お試しください。 |
| Cloud Sync | クラウド同期 |
| Restore from Cloud | クラウドから復元 |
| Deploy a Worker from Hatoba | Hatoba から Worker をデプロイ |
| Connect a deployed Worker | デプロイ済みの Worker に接続 |
| Account ID | Account ID |
| workers.dev subdomain | workers.dev サブドメイン |
| Check again | もう一度確認 |
| Remove what Hatoba created | Hatoba が作成したものを削除 |
| Check the token and account | トークンとアカウントを確認 |
| Look for an existing Worker and database | 既存の Worker とデータベースを確認 |
| Create the database | データベースを作成 |
| Create the tables | テーブルを作成 |
| Upload the Worker | Worker をアップロード |
| Set the setup token | Setup Token を設定 |
| Turn on the workers.dev URL | workers.dev の URL を有効化 |
| Wait for the Worker to answer | Worker の応答を待機 |
| Create a token in Cloudflare | Cloudflare でトークンを作成 |
| Edit token in Cloudflare | Cloudflare でトークンを編集 |
| Account ID (optional) | Account ID（任意） |
| Worker and database names | Worker とデータベースの名前 |
| Retry | 再試行 |

### Devices and conflicts

| en | ja |
|---|---|
| {n} conflicts were resolved automatically | {n} 件の競合を自動的に解決しました |

### Still not solved

| en | ja |
|---|---|
| Report a Problem | 問題を報告 |
| Review | 確認 |
| Devices | デバイス |
| Remove | 削除 |
