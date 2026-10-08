# 部署 Hatoba Sync Worker

> English: the project [README](../../README.md#set-up-sync) gives the deployment steps in English.

Hatoba 的云同步服务端是一个运行在**你自己的 Cloudflare 账号**里的 Worker（Hono）加一个 D1 数据库，不依赖任何 Hatoba 官方服务器。一个部署只服务一个用户（单保险库），服务端只保存密文，拿不到主密码、`vault_key` 或任何明文。

本页给出部署、升级和维护的步骤。接口与服务端行为见[架构与需求文档 §6.2](../../docs/hatoba-spec.md#62-worker-api)，安全属性见 [§4.4 威胁模型](../../docs/hatoba-spec.md#44-威胁模型)和 [§4.5 服务端可见的数据](../../docs/hatoba-spec.md#45-服务端可见的数据)，本地开发与测试见[开发指南](../../docs/development.md#同步-worker)。

## 准备

需要：

- 一个 Cloudflare 账号（免费版即可，个人使用通常在免费额度内）
- Node.js **22 或更新**（wrangler 4 的要求）和 npm

以下命令均在本目录（`workers/sync`）执行，Windows PowerShell 与 macOS / Linux 通用。

## 部署

### 1. 安装依赖并登录

```sh
npm install
npx wrangler login
```

### 2. 创建 D1 数据库

```sh
npx wrangler d1 create hatoba
```

可以用 `--location apac|weur|eeur|oc|wnam|enam` 指定离你近的区域，例如 `npx wrangler d1 create hatoba --location apac`。

命令会打印一个 `database_id`。把它填进 [`wrangler.toml`](./wrangler.toml) 的 `[[d1_databases]]`，替换占位值 `00000000-0000-0000-0000-000000000000`：

```toml
[[d1_databases]]
binding = "DB"                  # 不要改，代码依赖这个名字
database_name = "hatoba"
database_id = "<这里填 wrangler 打印的 id>"
migrations_dir = "migrations"
```

如果 wrangler 询问是否自动把绑定写入配置，选择"否"并手动填写；若选了"是"，请确认配置里没有重复的 `[[d1_databases]]` 段。

### 3. 建表（迁移）

```sh
npx wrangler d1 migrations apply hatoba --remote
```

没有 `--remote` 时只会作用于本地开发数据库。

### 4. 设置 Setup Token

Setup Token 用来防止别人抢在你之前初始化一个刚部署、还没配置的 Worker。它是一个只有你知道的随机字符串，**请用强随机值**：

```sh
# macOS / Linux / Git Bash
openssl rand -base64 32

# 任意系统（只要装了 Node）
node -e "console.log(require('crypto').randomBytes(32).toString('base64url'))"

# Windows PowerShell（不需要 openssl）
$b = New-Object byte[] 32; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b); [Convert]::ToBase64String($b)
```

把生成的值存进密码管理器（稍后要在 Hatoba 里粘贴），然后写入 Worker 的 secret：

```sh
npx wrangler secret put SETUP_TOKEN
```

按提示粘贴该值。如果 wrangler 提示"还没有名为 hatoba-sync 的 Worker，是否创建"，回答是。Secret 不会出现在 `wrangler.toml` 或 git 里。

### 5. 部署

```sh
npx wrangler deploy
```

输出里会有 Worker 的地址，形如 `https://hatoba-sync.<你的子域>.workers.dev`。验证部署是否成功：

```sh
curl https://hatoba-sync.<你的子域>.workers.dev/v1/health
```

返回的 JSON 中 `service` 为 `"hatoba-sync"`、`initialized` 为 `false`，表示部署成功、尚未初始化。如果返回 `503 database_unavailable`，说明第 2 步的 `database_id` 填错了或第 3 步没有执行。

### 6. 在 Hatoba 里启用同步

1. 在 Hatoba 的侧边栏打开**云同步**，选择**部署 Worker**。
2. 填写 **Worker URL**（第 5 步的地址）和 **Setup Token**（第 4 步生成的值），点击"测试连接"。
3. 按向导设置或输入主密码。第一台设备会调用 `/v1/setup` 完成初始化，随后自动登录并推送全部条目。

其他设备加入：在新设备首次启动时选择"从云端恢复"，只需要 Worker URL 和主密码，**不需要** Setup Token。

## 维护

### 初始化之后（可选加固）

初始化完成后 Setup Token 就不再需要了。你可以删除它，这样 `/v1/setup` 会一直返回 `503 setup_token_not_configured`：

```sh
npx wrangler secret delete SETUP_TOKEN
```

### 升级

```sh
git pull
npm install
npx wrangler d1 migrations apply hatoba --remote   # 只会执行新增的迁移
npx wrangler deploy
```

### 重置（丢弃云端保险库）

只在你确定要清空云端数据（例如初始化时填错了东西）时使用，本地设备上的数据不受影响：

```sh
npx wrangler d1 execute hatoba --remote --command "DELETE FROM sessions; DELETE FROM items; DELETE FROM meta;"
```

### 备份与检查

```sh
npx wrangler d1 export hatoba --remote --output hatoba-backup.sql
```

导出文件里只有密文、KDF 参数和哈希，搜不到任何主机名、密码或私钥。这也是检查"服务端只有密文"的办法。
