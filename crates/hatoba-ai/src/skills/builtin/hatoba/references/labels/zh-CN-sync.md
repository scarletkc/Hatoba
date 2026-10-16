# Simplified Chinese labels: Cloud Sync and the sync Worker

The labels that these files name in English, as the interface shows them in Simplified Chinese (zh-CN). Quote them exactly; the same English label can read differently in another file or section. A word in braces, such as {host}, stands for the value the interface fills in.

## `references/sync.md`

| en | zh-CN |
|---|---|
| Cloud Sync | 云同步 |
| Vault | 保险库 |
| Sync is off | 未开启云同步 |
| Stored on this device only | 数据仅保存在本机 |
| Synced | 已同步 |
| Syncing… | 正在同步… |
| Offline | 离线 |
| Will retry automatically | 将自动重试 |
| Signed out | 认证失效 |
| Enter your master password again | 请重新输入主密码 |
| Sync error | 同步出错 |
| Worker update required | 需要更新 Worker |
| Hatoba update required | 需要更新 Hatoba |
| Resolved automatically, review pending | 已自动解决，等待查看 |

### Set up on the first device

| en | zh-CN |
|---|---|
| Method | 接入方式 |
| Connect | 连接 |
| Password | 主密码 |
| Deploy a Worker from Hatoba | 从 Hatoba 部署 Worker |
| Recommended | 推荐 |
| Connect a deployed Worker | 连接已部署的 Worker |
| Use an API Token | 直接填写 API Token |
| How is my data encrypted? | 数据如何加密？ |
| Continue | 继续 |
| Cancel | 取消 |
| Back | 返回 |
| Enter Your Master Password | 输入主密码 |
| Master Password | 主密码 |
| Finish Setup | 完成设置 |
| Cloud Sync is on | 云同步已开启 |
| Uploading {n} items… | 正在上传 {n} 项… |

### Join a cloud vault

| en | zh-CN |
|---|---|
| Connected · the cloud already holds a vault | 已连接 · 云端已有保险库 |
| Enter the Cloud Vault’s Master Password | 输入云端保险库的主密码 |
| Merge and Turn On Sync | 合并并开启同步 |
| Merging… | 正在合并… |

### Add another device

| en | zh-CN |
|---|---|
| Restore from Cloud | 从云端恢复 |
| Method | 接入方式 |
| Sync Worker | 同步 Worker |
| Direct D1 | D1 直连 |
| Worker URL | Worker URL |
| Test Connection | 测试连接 |
| Restore Vault | 恢复保险库 |

### The status page

| en | zh-CN |
|---|---|
| Sync Now | 立即同步 |
| Connection | 连接 |
| Endpoint | 端点 |
| Database | 数据库 |
| Sync automatically | 自动同步 |
| Devices | 设备 |
| This PC | 本机 |
| Not seen in 14+ days | 超过 14 天未同步 |
| Remove | 移除 |
| Sign In Again | 重新登录 |
| Remove Device | 移除设备 |

### Conflicts

| en | zh-CN |
|---|---|
| Review | 查看 |
| Keep Current Result | 保留当前结果 |
| Restore Other Version | 恢复另一版本 |
| Cloud | 云端 |
| No conflicts to review. | 没有需要查看的冲突。 |
| Kept both versions; one is marked “(conflict copy)” | 已保留两个版本，其中一份标为“（冲突副本）” |

### Security actions and disconnecting

| en | zh-CN |
|---|---|
| Always ask | 始终询问 |
| Generate New Recovery Code | 生成新的恢复码 |
| Change Master Password | 更改主密码 |
| Disconnect… | 断开云同步… |
| Disconnect Cloud Sync? | 断开云同步？ |
| Disconnect | 断开 |

## `references/sync-worker.md`

### Method 1: Deploy a Worker from Hatoba

| en | zh-CN |
|---|---|
| Connect | 连接 |
| Deploy with an API Token | 用 API Token 部署 |
| The token needs two permissions | Token 需要两项权限 |
| Create a token in Cloudflare | 在 Cloudflare 中创建 Token |
| API Token | API Token |
| Account ID (optional) | Account ID（可选） |
| Review the Deployment | 确认部署内容 |
| Cloudflare account | Cloudflare 账户 |
| workers.dev subdomain | workers.dev 子域名 |
| Worker and database names | Worker 和数据库名称 |
| Worker name | Worker 名称 |
| Database name | 数据库名称 |
| Deploy | 部署 |
| Continue | 继续 |
| Check the token and account | 检查 Token 和账户 |
| Look for an existing Worker and database | 查找已有的 Worker 和数据库 |
| Create the database | 创建数据库 |
| Create the tables | 创建数据表 |
| Upload the Worker | 上传 Worker |
| Set the setup token | 设置 Setup Token |
| Turn on the workers.dev URL | 启用 workers.dev 地址 |
| Wait for the Worker to answer | 等待 Worker 响应 |
| not needed | 无需操作 |
| Sync Worker Deployed | 同步 Worker 已部署 |
| Deployment Stopped | 部署没有完成 |
| Retry | 重试 |
| Remove what Hatoba created | 移除 Hatoba 创建的内容 |
| Check again | 再次检查 |
| Back | 返回 |

### Method 2: Connect a deployed Worker

| en | zh-CN |
|---|---|
| Deploy the Sync Worker | 部署同步 Worker |
| One-click deploy | 一键部署 |
| Deploy to Cloudflare | 部署到 Cloudflare |
| Deploy manually with wrangler | 使用 wrangler 手动部署 |
| Full deployment guide | 完整部署说明 |
| Worker URL | Worker URL |
| Setup Token | Setup Token |
| Test Connection | 测试连接 |
| Connected | 已连接 |

### Method 3: Use an API Token (direct D1)

| en | zh-CN |
|---|---|
| Enter an API Token | 填写 API Token |
| Account ID | Account ID |
| Verify Token | 验证 Token |
| Database | 数据库 |
| Choose a database | 选择数据库 |
| Edit token in Cloudflare | 在 Cloudflare 中编辑 Token |

### Upgrading the Worker

| en | zh-CN |
|---|---|
| Update Worker | 更新 Worker |
| Dismiss | 忽略 |
| Check for updates | 检查更新 |
| Update the Sync Worker | 更新同步 Worker |
| Review the Update | 确认更新内容 |
| Update | 更新 |
| Updating the Sync Worker | 正在更新同步 Worker |
| Sync Worker Updated | 同步 Worker 已更新 |
| Update Stopped | 更新没有完成 |
| Check the Worker and its database | 检查 Worker 和它的数据库 |
| Update the tables | 更新数据表 |
| Upgrade steps | 升级步骤 |
| Worker update required | 需要更新 Worker |
| Hatoba update required | 需要更新 Hatoba |

## `references/troubleshooting-sync.md`

### Sync states

| en | zh-CN |
|---|---|
| Offline | 离线 |
| Will retry automatically | 将自动重试 |
| Signed out | 认证失效 |
| Enter your master password again | 请重新输入主密码 |
| Sync error | 同步出错 |
| Worker update required | 需要更新 Worker |
| Hatoba update required | 需要更新 Hatoba |
| Sync is off | 未开启云同步 |
| Stored on this device only | 数据仅保存在本机 |
| Sign In Again | 重新登录 |
| Sync Now | 立即同步 |
| Endpoint | 端点 |
| Update Worker | 更新 Worker |
| Check for Updates | 检查更新 |

### Messages

| en | zh-CN |
|---|---|
| Sync failed. | 同步失败。 |
| Can’t reach the sync service. Check your network. | 无法连接到同步服务，请检查网络。 |
| Cloud Sync signed out. Enter your master password again. | 云同步认证失效，请重新输入主密码。 |
| The cloud already holds a vault. Go back and connect again to join it. | 云端已经有一个保险库。返回上一步重新连接，即可加入它。 |
| The cloud has no vault yet. Turn on Cloud Sync on a device that has your data first. | 云端还没有保险库。请先在已有数据的设备上开启云同步。 |
| Another Cloud Sync setup is still running. Try again when it finishes. | 另一项云同步设置还在进行，请等它完成后再试。 |
| Cloud Sync is already on for this device. Disconnect it before joining another cloud vault. | 这台设备已经开启了云同步。要加入其他云端保险库，请先断开云同步。 |
| Connection failed. Check the URL and your network. | 连接失败，请检查地址和网络。 |
| Enter the full Worker address, e.g. https://hatoba-sync.example.workers.dev. | 请输入完整的 Worker 地址，例如 https://hatoba-sync.example.workers.dev。 |
| Incorrect master password. | 主密码不正确。 |
| Cancelled. | 已取消。 |

### An item too large to sync

| en | zh-CN |
|---|---|
| Your settings are too large to sync. Shorten your custom instructions. | 设置的数据太大，无法同步。请缩短自定义指令。 |
| This host is too large to sync. Shorten its notes or environment variables. | 这台主机的数据太大，无法同步。请缩短备注或环境变量。 |
| Edit Host | 编辑主机 |
| Open Settings → AI | 打开设置 → AI |
| Custom Instructions | 自定义指令 |
| Host “{name}” is too large to sync. Shorten its notes, AI notes, or environment variables. | 主机“{name}”的数据太大，无法同步。请缩短它的备注、AI 备注或环境变量。 |

### Cloudflare and deployment errors

| en | zh-CN |
|---|---|
| Cloudflare didn’t accept this API token. Check that it’s complete and not expired or disabled. An account-owned token also needs the account ID. | Cloudflare 不接受这个 API Token。请检查它是否完整、是否已过期或停用；账户所属的 Token 还需要填写 Account ID。 |
| Cloudflare returned an error. Try again later. | Cloudflare 返回了错误，请稍后重试。 |
| Choose a workers.dev subdomain for this account. | 请为这个账户选择一个 workers.dev 子域名。 |
| This workers.dev subdomain is taken. Choose another. | 这个 workers.dev 子域名已被占用，请换一个。 |
| Another Worker already has this name, and it isn’t a Hatoba sync Worker. Choose another name. | 已有同名的 Worker，但它不是 Hatoba 的同步 Worker。请换一个名称。 |
| This account has no Hatoba Worker with this name, or it holds no vault. Check the account and the Worker name. | 这个账户中找不到这个名称的 Hatoba Worker，或者它没有保存保险库。请检查账户和 Worker 名称。 |
| This Worker is newer than the one in this version of Hatoba, and Hatoba won’t replace it with an older one. | 这个 Worker 比这个版本的 Hatoba 内置的更新，Hatoba 不会用旧版本替换它。 |
| This build of Hatoba doesn’t include the sync Worker, so it can’t deploy it. Use another way to deploy. | 这个版本的 Hatoba 没有内置同步 Worker，无法一键部署。请改用其他部署方式。 |
| This token can’t edit D1. Add “Account · D1 · Edit” to it in Cloudflare. | 这个 Token 没有 D1 编辑权限。请在 Cloudflare 中为它添加「Account · D1 · Edit」。 |
| Can’t reach Cloudflare. Check your network and try again. | 无法连接到 Cloudflare，请检查网络后重试。 |
| Cloud Sync | 云同步 |
| Restore from Cloud | 从云端恢复 |
| Deploy a Worker from Hatoba | 从 Hatoba 部署 Worker |
| Connect a deployed Worker | 连接已部署的 Worker |
| Account ID | Account ID |
| workers.dev subdomain | workers.dev 子域名 |
| Check again | 再次检查 |
| Remove what Hatoba created | 移除 Hatoba 创建的内容 |
| Check the token and account | 检查 Token 和账户 |
| Look for an existing Worker and database | 查找已有的 Worker 和数据库 |
| Create the database | 创建数据库 |
| Create the tables | 创建数据表 |
| Upload the Worker | 上传 Worker |
| Set the setup token | 设置 Setup Token |
| Turn on the workers.dev URL | 启用 workers.dev 地址 |
| Wait for the Worker to answer | 等待 Worker 响应 |
| Create a token in Cloudflare | 在 Cloudflare 中创建 Token |
| Edit token in Cloudflare | 在 Cloudflare 中编辑 Token |
| Account ID (optional) | Account ID（可选） |
| Worker and database names | Worker 和数据库名称 |
| Retry | 重试 |

### Devices and conflicts

| en | zh-CN |
|---|---|
| {n} conflicts were resolved automatically | {n} 项冲突已自动解决 |

### Still not solved

| en | zh-CN |
|---|---|
| Report a Problem | 反馈问题 |
| Review | 查看 |
| Devices | 设备 |
| Remove | 移除 |
