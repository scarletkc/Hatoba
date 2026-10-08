---
kind: howto
---

# 开发指南

在本地构建、运行和测试 Hatoba。CI 执行的完整检查见 [`.github/workflows/ci.yml`](../.github/workflows/ci.yml)。

## 环境要求

- Rust stable、Node.js 22+、pnpm（版本见根目录 [`package.json`](../package.json) 的 `packageManager`）。
- Windows：WebView2（Windows 11 自带）。
- Linux：`libwebkit2gtk-4.1-dev` 等 Tauri 依赖，CI 安装的完整列表见 `ci.yml` 中 `rust-linux` 任务的 System dependencies 步骤。
- SSH 集成测试和端到端测试需要 OpenSSH 服务器（`sshd`）。

## 运行

```sh
pnpm install
pnpm tauri dev            # 启动桌面应用
pnpm dev                  # 只启动前端，在浏览器中使用模拟后端
```

`pnpm dev` 在浏览器中运行时使用 `apps/desktop/src/ipc/mock` 的模拟后端，带设计稿示例数据。URL 参数可以切换演示状态，例如 `?state=locked`、`?sync=conflict`；完整的参数列表见 `apps/desktop/src/ipc/mock/index.ts` 中 `createMockApi` 的注释。

## 测试

`hatoba-desktop` 编译时会嵌入前端构建产物，所以第一次运行 `cargo` 命令前先构建一次前端：

```sh
pnpm build
```

然后：

```sh
cargo test --workspace         # Rust 单元测试
pnpm typecheck && pnpm test    # 前端类型检查与单元测试
```

各项测试覆盖的范围见[架构与需求文档 §12](hatoba-spec.md#12-测试)。

### SSH 集成测试

这些测试会启动一个临时的本机 `sshd`，需要已安装 OpenSSH 服务器；没有设置 `HATOBA_SSH_IT=1` 时跳过。

```sh
HATOBA_SSH_IT=1 cargo test -p hatoba-ssh
```

### 同步 Worker

在 `workers/sync` 目录执行：

```sh
npm install
npm run typecheck
npm test               # Vitest，在本地 workerd + 本地 D1 上运行全部接口测试
```

每个测试前清空本地 D1，迁移在测试启动时自动应用。

手动调试：

```sh
cp .dev.vars.example .dev.vars        # 里面的 SETUP_TOKEN 只用于本地
npm run db:migrate:local
npm run dev                           # http://localhost:8787
```

本地 `wrangler dev` 没有 `CF-Connecting-IP` 请求头，所有请求共用同一个限流计数。

### 客户端与 Worker 联调

`crates/hatoba-core/tests/worker_live.rs` 让 Rust 客户端与 `wrangler dev` 运行的真实 Worker 同步，没有设置 `HATOBA_WORKER_URL` 时跳过。启动 Worker 和运行测试的命令写在该文件开头的注释里。

### 端到端冒烟测试

只支持 Linux，步骤见[端到端冒烟测试](../apps/desktop/e2e/README.md)。

## 更新 TypeScript 绑定

修改了 Rust 端的 command、事件或 DTO 之后，重新生成 `apps/desktop/src/ipc/bindings.ts`：

```sh
cargo test -p hatoba-desktop export_bindings
```

`pnpm typecheck` 会通过 `apps/desktop/src/ipc/contract.check.ts` 比对生成的绑定与前端使用的契约；CI 在绑定没有更新时失败。

## 提交前检查

CI 还会运行格式和 lint 检查：

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## 常见问题

- 在 `workers/sync` 添加依赖时，npm 10 可能报 `Cannot read properties of null (reading 'edgesOut')`。这是 npm 10.x 解析 vitest 可选 peer 依赖时的已知问题，升级到 npm 11 或改用 pnpm 即可。直接 `npm install` / `npm ci` 已有的 `package-lock.json` 不受影响。
