# Traditional Chinese labels: Cloud Sync and the sync Worker

The labels that these files name in English, as the interface shows them in Traditional Chinese (zh-TW). Quote them exactly; the same English label can read differently in another file or section. A word in braces, such as {host}, stands for the value the interface fills in.

## `references/sync.md`

| en | zh-TW |
|---|---|
| Sync | 雲端同步 |
| Cloud Sync | 雲端同步 |
| Sync is off | 未開啟雲端同步 |
| Stored on this device only | 資料僅儲存在本機 |
| Synced | 已同步 |
| Syncing… | 正在同步… |
| Offline | 離線 |
| Will retry automatically | 將自動重試 |
| Signed out | 驗證失效 |
| Enter your master password again | 請重新輸入主密碼 |
| Sync error | 同步錯誤 |
| Worker update required | 需要更新 Worker |
| Hatoba update required | 需要更新 Hatoba |
| Resolved automatically, review pending | 已自動解決，等待檢視 |

### Set up on the first device

| en | zh-TW |
|---|---|
| Method | 連線方式 |
| Connect | 連線 |
| Password | 主密碼 |
| Deploy a Worker from Hatoba | 從 Hatoba 部署 Worker |
| Recommended | 建議 |
| Connect a deployed Worker | 連線到已部署的 Worker |
| Use an API Token | 直接填寫 API 權杖 |
| How is my data encrypted? | 資料如何加密？ |
| Continue | 繼續 |
| Cancel | 取消 |
| Back | 返回 |
| Enter Your Master Password | 輸入主密碼 |
| Master Password | 主密碼 |
| Finish Setup | 完成設定 |
| Cloud Sync is on | 雲端同步已開啟 |
| Uploading {n} items… | 正在上傳 {n} 項… |

### Join a cloud vault

| en | zh-TW |
|---|---|
| Connected · the cloud already holds a vault | 已連線 · 雲端已有保險庫 |
| Enter the Cloud Vault’s Master Password | 輸入雲端保險庫的主密碼 |
| Merge and Turn On Sync | 合併並開啟同步 |
| Merging… | 正在合併… |

### Add another device

| en | zh-TW |
|---|---|
| Restore from Cloud | 從雲端還原 |
| Method | 連線方式 |
| Sync Worker | 同步 Worker |
| Direct D1 | D1 直接連線 |
| Worker URL | Worker URL |
| Test Connection | 測試連線 |
| Restore Vault | 還原保險庫 |

### The status page

| en | zh-TW |
|---|---|
| Sync Now | 立即同步 |
| Connection | 連線 |
| Endpoint | 端點 |
| Database | 資料庫 |
| Sync automatically | 自動同步 |
| Devices | 裝置 |
| This PC | 本機 |
| Not seen in 14+ days | 超過 14 天未同步 |
| Remove | 移除 |
| Sign In Again | 重新登入 |
| Remove Device | 移除裝置 |

### Conflicts

| en | zh-TW |
|---|---|
| Review | 檢視 |
| Keep Current Result | 保留目前結果 |
| Restore Other Version | 還原另一版本 |
| Cloud | 雲端 |
| No conflicts to review. | 沒有需要檢視的衝突。 |
| Kept both versions; one is marked “(conflict copy)” | 已保留兩個版本，其中一份標為「（衝突副本）」 |

### Security actions and disconnecting

| en | zh-TW |
|---|---|
| Always ask | 一律詢問 |
| Generate New Recovery Code | 產生新的復原碼 |
| Change Master Password | 變更主密碼 |
| Disconnect | 中斷雲端同步 |
| Disconnect Cloud Sync? | 中斷雲端同步？ |
| Disconnect | 中斷連線 |

## `references/sync-worker.md`

### Method 1: Deploy a Worker from Hatoba

| en | zh-TW |
|---|---|
| Connect | 連線 |
| Deploy with an API Token | 用 API 權杖部署 |
| The token needs two permissions | 權杖需要兩項權限 |
| Create a token in Cloudflare | 在 Cloudflare 中建立權杖 |
| API Token | API 權杖 |
| Account ID (optional) | Account ID（選填） |
| Review the Deployment | 確認部署內容 |
| Cloudflare account | Cloudflare 帳戶 |
| workers.dev subdomain | workers.dev 子網域 |
| Worker and database names | Worker 和資料庫名稱 |
| Worker name | Worker 名稱 |
| Database name | 資料庫名稱 |
| Deploy | 部署 |
| Continue | 繼續 |
| Check the token and account | 檢查權杖和帳戶 |
| Look for an existing Worker and database | 尋找現有的 Worker 和資料庫 |
| Create the database | 建立資料庫 |
| Create the tables | 建立資料表 |
| Upload the Worker | 上傳 Worker |
| Set the setup token | 設定 Setup Token |
| Turn on the workers.dev URL | 啟用 workers.dev 位址 |
| Wait for the Worker to answer | 等待 Worker 回應 |
| not needed | 不需要 |
| Sync Worker Deployed | 同步 Worker 已部署 |
| Deployment Stopped | 部署沒有完成 |
| Retry | 重試 |
| Remove what Hatoba created | 移除 Hatoba 建立的內容 |
| Check again | 再次檢查 |
| Back | 返回 |

### Method 2: Connect a deployed Worker

| en | zh-TW |
|---|---|
| Deploy the Sync Worker | 部署同步 Worker |
| One-click deploy | 一鍵部署 |
| Deploy to Cloudflare | 部署到 Cloudflare |
| Deploy manually with wrangler | 使用 wrangler 手動部署 |
| Full deployment guide | 完整部署說明 |
| Worker URL | Worker URL |
| Setup Token | Setup Token |
| Test Connection | 測試連線 |
| Connected | 已連線 |

### Method 3: Use an API Token (direct D1)

| en | zh-TW |
|---|---|
| Enter an API Token | 填寫 API 權杖 |
| Account ID | Account ID |
| Verify Token | 驗證權杖 |
| Database | 資料庫 |
| Choose a database | 選擇資料庫 |
| Edit token in Cloudflare | 在 Cloudflare 中編輯權杖 |

### Upgrading the Worker

| en | zh-TW |
|---|---|
| Update Worker | 更新 Worker |
| Dismiss | 忽略 |
| Check for updates | 檢查更新 |
| Update the Sync Worker | 更新同步 Worker |
| Review the Update | 確認更新內容 |
| Update | 更新 |
| Updating the Sync Worker | 正在更新同步 Worker |
| Sync Worker Updated | 同步 Worker 已更新 |
| Update Stopped | 更新沒有完成 |
| Check the Worker and its database | 檢查 Worker 和它的資料庫 |
| Update the tables | 更新資料表 |
| Upgrade steps | 升級步驟 |
| Worker update required | 需要更新 Worker |
| Hatoba update required | 需要更新 Hatoba |

## `references/troubleshooting-sync.md`

### Sync states

| en | zh-TW |
|---|---|
| Offline | 離線 |
| Will retry automatically | 將自動重試 |
| Signed out | 驗證失效 |
| Enter your master password again | 請重新輸入主密碼 |
| Sync error | 同步錯誤 |
| Worker update required | 需要更新 Worker |
| Hatoba update required | 需要更新 Hatoba |
| Sync is off | 未開啟雲端同步 |
| Stored on this device only | 資料僅儲存在本機 |
| Sign In Again | 重新登入 |
| Sync Now | 立即同步 |
| Endpoint | 端點 |
| Update Worker | 更新 Worker |
| Check for Updates | 檢查更新 |

### Messages

| en | zh-TW |
|---|---|
| Sync failed. | 同步失敗。 |
| Can’t reach the sync service. Check your network. | 無法連線到同步服務，請檢查網路。 |
| Cloud Sync signed out. Enter your master password again. | 雲端同步驗證失效，請重新輸入主密碼。 |
| The cloud already holds a vault. Go back and connect again to join it. | 雲端已經有一個保險庫。返回上一步重新連線，即可加入它。 |
| The cloud has no vault yet. Turn on Cloud Sync on a device that has your data first. | 雲端還沒有保險庫。請先在已有資料的裝置上開啟雲端同步。 |
| Another Cloud Sync setup is still running. Try again when it finishes. | 另一項雲端同步設定還在進行，請等它完成後再試。 |
| Cloud Sync is already on for this device. Disconnect it before joining another cloud vault. | 這台裝置已經開啟了雲端同步。要加入其他雲端保險庫，請先中斷雲端同步。 |
| Connection failed. Check the URL and your network. | 連線失敗，請檢查位址和網路。 |
| Enter the full Worker address, e.g. https://hatoba-sync.example.workers.dev. | 請輸入完整的 Worker 位址，例如 https://hatoba-sync.example.workers.dev。 |
| Incorrect master password. | 主密碼不正確。 |
| Cancelled. | 已取消。 |

### An item too large to sync

| en | zh-TW |
|---|---|
| Your settings are too large to sync. Shorten your custom instructions. | 設定的資料太大，無法同步。請縮短自訂指令。 |
| This host is too large to sync. Shorten its notes or environment variables. | 這台主機的資料太大，無法同步。請縮短備註或環境變數。 |
| Edit Host | 編輯主機 |
| Open Settings → AI | 開啟設定 → AI |
| Custom Instructions | 自訂指令 |
| Host “{name}” is too large to sync. Shorten its notes, AI notes, or environment variables. | 主機「{name}」的資料太大，無法同步。請縮短它的備註、AI 備註或環境變數。 |

### Cloudflare and deployment errors

| en | zh-TW |
|---|---|
| Cloudflare didn’t accept this API token. Check that it’s complete and not expired or disabled. An account-owned token also needs the account ID. | Cloudflare 不接受這個 API 權杖。請檢查它是否完整、是否已過期或停用；帳戶所屬的權杖還需要填寫 Account ID。 |
| Cloudflare returned an error. Try again later. | Cloudflare 傳回了錯誤，請稍後重試。 |
| Choose a workers.dev subdomain for this account. | 請為這個帳戶選擇一個 workers.dev 子網域。 |
| This workers.dev subdomain is taken. Choose another. | 這個 workers.dev 子網域已被佔用，請換一個。 |
| Another Worker already has this name, and it isn’t a Hatoba sync Worker. Choose another name. | 已有同名的 Worker，但它不是 Hatoba 的同步 Worker。請換一個名稱。 |
| This account has no Hatoba Worker with this name, or it holds no vault. Check the account and the Worker name. | 這個帳戶中找不到這個名稱的 Hatoba Worker，或者它沒有儲存保險庫。請檢查帳戶和 Worker 名稱。 |
| This Worker is newer than the one in this version of Hatoba, and Hatoba won’t replace it with an older one. | 這個 Worker 比這個版本的 Hatoba 內建的更新，Hatoba 不會用舊版本取代它。 |
| This build of Hatoba doesn’t include the sync Worker, so it can’t deploy it. Use another way to deploy. | 這個版本的 Hatoba 沒有內建同步 Worker，無法一鍵部署。請改用其他部署方式。 |
| This token can’t edit D1. Add “Account · D1 · Edit” to it in Cloudflare. | 這個權杖沒有 D1 編輯權限。請在 Cloudflare 中為它新增「Account · D1 · Edit」。 |
| Can’t reach Cloudflare. Check your network and try again. | 無法連線到 Cloudflare，請檢查網路後重試。 |
| Cloud Sync | 雲端同步 |
| Restore from Cloud | 從雲端還原 |
| Deploy a Worker from Hatoba | 從 Hatoba 部署 Worker |
| Connect a deployed Worker | 連線到已部署的 Worker |
| Account ID | Account ID |
| workers.dev subdomain | workers.dev 子網域 |
| Check again | 再次檢查 |
| Remove what Hatoba created | 移除 Hatoba 建立的內容 |
| Check the token and account | 檢查權杖和帳戶 |
| Look for an existing Worker and database | 尋找現有的 Worker 和資料庫 |
| Create the database | 建立資料庫 |
| Create the tables | 建立資料表 |
| Upload the Worker | 上傳 Worker |
| Set the setup token | 設定 Setup Token |
| Turn on the workers.dev URL | 啟用 workers.dev 位址 |
| Wait for the Worker to answer | 等待 Worker 回應 |
| Create a token in Cloudflare | 在 Cloudflare 中建立權杖 |
| Edit token in Cloudflare | 在 Cloudflare 中編輯權杖 |
| Account ID (optional) | Account ID（選填） |
| Worker and database names | Worker 和資料庫名稱 |
| Retry | 重試 |

### Devices and conflicts

| en | zh-TW |
|---|---|
| {n} conflicts were resolved automatically | {n} 項衝突已自動解決 |

### Still not solved

| en | zh-TW |
|---|---|
| Report a Problem | 回報問題 |
| Review | 檢視 |
| Devices | 裝置 |
| Remove | 移除 |
