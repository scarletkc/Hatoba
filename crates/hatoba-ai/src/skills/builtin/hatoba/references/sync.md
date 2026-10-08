# Cloud Sync

Cloud Sync keeps the vault the same on several devices. It is optional and off until set up. Hatoba has no sync server: the data goes to a Worker and a D1 database in the user's own Cloudflare account (or straight to D1), always encrypted on the device first, so the cloud holds ciphertext only. Setting up, deploying and upgrading the Worker is in `references/sync-worker.md`.

Open it from the sidebar (**Cloud Sync**, under **Vault**). The sync button at the bottom of the sidebar shows the current state and opens the same page.

| en | zh-CN | ja |
|---|---|---|
| Cloud Sync | 云同步 | クラウド同期 |
| Vault | 保险库 | 保管庫 |
| Sync is off | 未开启云同步 | クラウド同期はオフ |
| Stored on this device only | 数据仅保存在本机 | このデバイスにのみ保存 |
| Synced | 已同步 | 同期済み |
| Syncing… | 正在同步… | 同期中… |
| Offline | 离线 | オフライン |
| Will retry automatically | 将自动重试 | 自動的に再試行します |
| Signed out | 认证失效 | 認証切れ |
| Enter your master password again | 请重新输入主密码 | マスターパスワードを再入力してください |
| Sync error | 同步出错 | 同期エラー |
| Worker update required | 需要更新 Worker | Worker の更新が必要 |
| Hatoba update required | 需要更新 Hatoba | Hatoba の更新が必要 |
| Resolved automatically, review pending | 已自动解决，等待查看 | 自動的に解決済み・確認待ち |

## What syncs

- Synced (encrypted): hosts and saved passwords, groups, SSH keys with their passphrases, trusted host fingerprints, port forwards, terminal settings, Auto-Lock settings, AI providers with their API keys and models, the search provider, the default model, AI conversations, skills, and MCP server definitions (with the **Always ask** setting).
- Not synced, kept on each device: language, theme, list density, host reachability, the update-check switch, the AI permission mode, the tool call limit and the AI panel size, whether an MCP server is switched on for the device, and the MCP "Always allow" switches.
- A sync round runs after unlock, about 2 seconds after a local change, every 60 seconds, when the window regains focus, and on **Sync Now**. Local work never waits for it.

## Set up on the first device

With sync off, the page is a three-step wizard (**Method**, **Connect**, **Password**). A vault must already exist locally.

| en | zh-CN | ja |
|---|---|---|
| Method | 接入方式 | 接続方法 |
| Connect | 连接 | 接続 |
| Password | 主密码 | パスワード |
| Deploy a Worker from Hatoba | 从 Hatoba 部署 Worker | Hatoba から Worker をデプロイ |
| Recommended | 推荐 | 推奨 |
| Connect a deployed Worker | 连接已部署的 Worker | デプロイ済みの Worker に接続 |
| Use an API Token | 直接填写 API Token | API トークンを直接入力 |
| How is my data encrypted? | 数据如何加密？ | データはどう暗号化される？ |
| Continue | 继续 | 続ける |
| Cancel | 取消 | キャンセル |
| Back | 返回 | 戻る |

1. **Method**: **Deploy a Worker from Hatoba** (recommended; Hatoba creates the Worker and database with a Cloudflare API token), **Connect a deployed Worker** (the user deployed it with the Deploy to Cloudflare button or wrangler and enters its URL and Setup Token), or **Use an API Token** (direct D1 mode: no Worker, the app calls the D1 API; the token can usually edit every D1 database in the account). **How is my data encrypted?** explains the encryption.
2. **Connect**: the details of the chosen method; see `references/sync-worker.md`.
3. **Password**: **Enter Your Master Password** (the existing one, not a new one), then **Finish Setup**. Hatoba uploads the vault ("Uploading N items…") and the page switches to the status page.

| en | zh-CN | ja |
|---|---|---|
| Enter Your Master Password | 输入主密码 | マスターパスワードを入力 |
| Master Password | 主密码 | マスターパスワード |
| Finish Setup | 完成设置 | 設定を完了 |
| Cloud Sync is on | 云同步已开启 | クラウド同期をオンにしました |

If the cloud already holds a vault, setup stops with "The cloud already has a vault. On a new device, choose “Restore from Cloud”." Connecting an existing local vault to a cloud vault that has other data is not supported.

## Add another device

On the new device's first launch choose **Restore from Cloud**: pick the method (**Sync Worker** or **Direct D1**), enter the Worker URL (or the account ID, token and database), use **Test Connection**, then enter the master password and **Restore Vault**. Data is downloaded and decrypted on the device. **Restore from Cloud** is offered only on first launch; a device that already has its own vault cannot be merged with a cloud vault in this version.

| en | zh-CN | ja |
|---|---|---|
| Restore from Cloud | 从云端恢复 | クラウドから復元 |
| Method | 接入方式 | 接続方式 |
| Sync Worker | 同步 Worker | 同期 Worker |
| Direct D1 | D1 直连 | D1 直接接続 |
| Worker URL | Worker URL | Worker URL |
| Test Connection | 测试连接 | 接続テスト |
| Restore Vault | 恢复保险库 | 保管庫を復元 |

## The status page

The header shows the state ("Synced", "Syncing…", "Offline", "Signed out", "Sync error", or a paused state) with the last sync time and counts of hosts, keys and groups, and the **Sync Now** button.

| en | zh-CN | ja |
|---|---|---|
| Sync Now | 立即同步 | 今すぐ同期 |
| Connection | 连接 | 接続 |
| Endpoint | 端点 | エンドポイント |
| Database | 数据库 | データベース |
| Sync automatically | 自动同步 | 自動同期 |
| Devices | 设备 | デバイス |
| This PC | 本机 | このPC |
| Not seen in 14+ days | 超过 14 天未同步 | 14 日以上同期されていません |
| Remove | 移除 | 削除 |

- **Connection** lists the **Method**, **Endpoint** (the Worker URL) and **Database**, and the **Sync automatically** switch. With it off, sync runs only on **Sync Now**.
- **Offline**: the network or the service is unreachable. Local changes are kept and uploaded automatically later. See `references/troubleshooting-sync.md`.
- **Signed out**: the session expired or was revoked. A master password field with **Sign In Again** appears on the page.
- A paused state (**Worker update required** or **Hatoba update required**) means the Worker and the app versions no longer match; see `references/sync-worker.md`.
- **Devices** lists the signed-in devices (**This PC** marks this one) with the platform and the last seen time. A device not seen for 14 days or more is flagged "Not seen in 14+ days". **Remove** next to another device (then **Remove Device**) revokes its sign-in. It needs the master password again to sync, and no data is deleted.

| en | zh-CN | ja |
|---|---|---|
| Sign In Again | 重新登录 | 再ログイン |
| Remove Device | 移除设备 | デバイスを削除 |

## Conflicts

When two devices edit the same item, Hatoba resolves it automatically: the newer edit wins; an edit beats a delete; for SSH keys the losing version is kept as a second key whose name carries a conflict-copy suffix. Sync does not stop. A banner on the status page says how many conflicts were resolved, with **Review**.

| en | zh-CN | ja |
|---|---|---|
| Review | 查看 | 確認 |
| Keep Current Result | 保留当前结果 | 現在の結果を維持 |
| Restore Other Version | 恢复另一版本 | もう一方を復元 |
| Cloud | 云端 | クラウド |
| No conflicts to review. | 没有需要查看的冲突。 | 確認が必要な競合はありません。 |
| Kept both versions; one is marked “(conflict copy)” | 已保留两个版本，其中一份标为“（冲突副本）” | 両方のバージョンを残し、片方に「（競合コピー）」を付けました |

The review page lists each item with **This PC** and **Cloud** versions. **Keep Current Result** accepts what Hatoba chose; **Restore Other Version** switches to the other side.

## Security actions and disconnecting

At the bottom of the status page:

| en | zh-CN | ja |
|---|---|---|
| Always ask | 始终询问 | 常に確認 |
| Generate New Recovery Code | 生成新的恢复码 | 新しいリカバリーコードを作成 |
| Change Master Password | 更改主密码 | マスターパスワードを変更 |
| Disconnect… | 断开云同步… | クラウド同期を解除… |
| Disconnect Cloud Sync? | 断开云同步？ | クラウド同期を解除しますか？ |
| Disconnect | 断开 | 解除 |

- **Generate New Recovery Code** and **Change Master Password** work like their Settings → Security counterparts. After a password change, the other devices are signed out and must sign in with the new one.
- **Disconnect…** stops syncing on this device. Data stays on the device and nothing is deleted from the cloud. You can reconnect through the wizard later.
