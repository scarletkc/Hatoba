# Development guide

Build, run, and test Hatoba locally. [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) has the full set of checks that CI runs.

## Requirements

- Rust stable, Node.js 22+, and pnpm (the version is in `packageManager` in the root [`package.json`](../package.json)).
- Windows: WebView2 (included in Windows 11).
- Linux: the Tauri dependencies, such as `libwebkit2gtk-4.1-dev`. The System dependencies step of the `rust-linux` job in `ci.yml` has the full list that CI installs.
- The SSH integration tests and the end-to-end test need an OpenSSH server (`sshd`).

## Run

```sh
pnpm install
pnpm tauri dev            # Start the desktop app
pnpm dev                  # Start only the frontend, with a mock backend in the browser
```

In the browser, `pnpm dev` uses the mock backend in `apps/desktop/src/ipc/mock` with the design's sample data. URL parameters switch demo states, such as `?state=locked` and `?sync=conflict`. The comment on `createMockApi` in `apps/desktop/src/ipc/mock/index.ts` lists every parameter.

## Testing

`hatoba-desktop` embeds the frontend build output at compile time, so build the frontend once before your first `cargo` command:

```sh
pnpm build
```

Then:

```sh
cargo test --workspace                # Rust unit tests
pnpm typecheck && pnpm test           # Frontend type check and unit tests
node --test "scripts/**/*.test.mjs"   # Release and CI scripts
```

[§12 of the architecture and requirements](hatoba-spec.md#12-testing) describes what each suite covers.

### SSH integration tests

These tests start a throwaway local `sshd` and need the OpenSSH server installed. They are skipped unless `HATOBA_SSH_IT=1` is set.

```sh
HATOBA_SSH_IT=1 cargo test -p hatoba-ssh
```

### Sync Worker

Run these in `workers/sync`:

```sh
npm install
npm run typecheck
npm test               # Vitest, runs every API test on local workerd with a local D1
```

Each test starts with an empty local D1, and the migrations are applied when the test run starts.

For manual debugging:

```sh
cp .dev.vars.example .dev.vars        # Its SETUP_TOKEN is for local use only
npm run db:migrate:local
npm run dev                           # http://localhost:8787
```

Local `wrangler dev` has no `CF-Connecting-IP` header, so all requests share one rate limit counter.

### In-app deployment

`pnpm tauri build` packages the Worker for [in-app deployment](hatoba-spec.md#67-in-app-deployment) with `scripts/worker/bundle.mjs`, and `build.rs` embeds the package. Other builds, such as `pnpm tauri dev` and `cargo test`, embed the package from the last run of `pnpm worker:bundle`, and without one the deploy commands report that the build has no Worker bundle. Run it again after changing the Worker. `pnpm worker:bundle --no-install` skips `npm ci` and uses the dependencies already installed in `workers/sync`.

### Client and Worker integration

`crates/hatoba-core/tests/worker_live.rs` syncs the Rust client with a real Worker running in `wrangler dev`, and is skipped unless `HATOBA_WORKER_URL` is set. The comment at the top of that file has the commands that start the Worker and run the test.

### End-to-end smoke test

Linux only. See [Run the end-to-end smoke test](../apps/desktop/e2e/README.md).

## Update the TypeScript bindings

After changing a Rust command, event, or DTO, regenerate `apps/desktop/src/ipc/bindings.ts`:

```sh
cargo test -p hatoba-desktop export_bindings
```

`pnpm typecheck` compares the generated bindings with the contract the frontend uses, through `apps/desktop/src/ipc/contract.check.ts`. CI fails when the bindings are out of date.

## Raise the Worker version

The sync Worker has a version of its own, separate from the app version. A pull request that changes the Worker raises it ([Versions in §6.7](hatoba-spec.md#worker-bundle)), and `workerInputs` in [`scripts/ci/worker-version.mjs`](../scripts/ci/worker-version.mjs) lists the files, dependencies, and `tsconfig.json` options that count. From the repository root:

```sh
node scripts/release/bump.mjs --worker patch   # Or minor, major, or an explicit version
node scripts/ci/worker-version.mjs             # Compare with where the branch left origin/main
```

The Sync Worker job in CI runs the same check against the base branch, and fails when the Worker changed but its version did not.

## Keep the built-in skill current

The AI assistant ships with a built-in `hatoba` skill in [`crates/hatoba-ai/src/skills/builtin/hatoba`](../crates/hatoba-ai/src/skills/builtin/hatoba). Its `SKILL.md` and `references/` files describe the current interface, settings, and features, so the assistant can answer questions about Hatoba. A pull request that adds, renames, or removes UI, settings, or a feature that the skill describes updates the skill in the same pull request.

- Each file stays under 15,000 characters, because `read_skill` shortens a longer file.
- Quote UI labels in tables headed `| en | zh-CN | ja |`, copied character for character from the message tables in `apps/desktop/src/i18n/locales`, and only from messages without `{placeholders}`.
- `{{HATOBA_VERSION}}` in `SKILL.md` is a placeholder that the app replaces with the running version.

`apps/desktop/src/i18n/hatobaSkill.test.ts` runs with `pnpm test`. It checks every table row against the message tables, the frontmatter, the size limit, and the links between the files.

## Checks before committing

CI also runs format and lint checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Troubleshooting

- When you add a dependency in `workers/sync`, npm 10 may fail with `Cannot read properties of null (reading 'edgesOut')`. This is a known npm 10.x issue with resolving the optional peer dependencies of vitest. Upgrade to npm 11 or use pnpm. A plain `npm install` or `npm ci` from the existing `package-lock.json` is not affected.
