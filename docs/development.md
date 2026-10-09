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

### Signed updates

The tests in `apps/desktop/src-tauri/src/update.rs` run the updater against a mock endpoint. To try an update end to end on Windows without publishing a release, install a test build and serve it a newer one from this machine. The test builds have their own name, identifier, and executable, so they install next to Hatoba without touching it or its vault.

1. Generate a throwaway key outside the repository:

   ```sh
   pnpm tauri signer generate --ci -p test -w <dir>/test.key
   ```

2. Write `<dir>/old.json`, with the contents of `<dir>/test.key.pub` as `pubkey`:

   ```json
   {
     "productName": "Hatoba Update Test",
     "mainBinaryName": "HatobaUpdateTest",
     "identifier": "app.hatoba.updatetest",
     "plugins": {
       "updater": {
         "pubkey": "<contents of test.key.pub>",
         "endpoints": ["http://127.0.0.1:18765/latest.json"],
         "dangerousInsecureTransportProtocol": true
       }
     }
   }
   ```

   Write `<dir>/new.json` with the same contents and a `"version"` higher than the one in `Cargo.toml`.

3. Build each version, and copy its installer and `.sig` out of `target/release/bundle/nsis` before the next build:

   ```sh
   export TAURI_SIGNING_PRIVATE_KEY=<dir>/test.key TAURI_SIGNING_PRIVATE_KEY_PASSWORD=test
   pnpm tauri build --bundles nsis --config <dir>/old.json
   pnpm tauri build --bundles nsis --config <dir>/new.json
   ```

4. Run the older installer, then serve the newer one:

   ```sh
   node scripts/release/serve-update.mjs --installer "<dir>/Hatoba Update Test_<version>_x64-setup.exe" --pubkey <dir>/test.key.pub
   ```

5. Open **Hatoba Update Test**, create a vault, and in **Settings → About** choose **Check for Updates**, then **Download and Install**. Hatoba closes, the installer shows its progress, and the new version opens.
6. Uninstall **Hatoba Update Test** in the Windows settings, delete `%LOCALAPPDATA%\app.hatoba.updatetest`, and delete the throwaway key.

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

## Update the README screenshots and demo video

The scripts in [`scripts/media`](../scripts/media) capture the screenshots and the demo video in `docs/media` from the browser frontend's mock backend. `?ai=showcase` in `apps/desktop/src/ipc/mock/ai.ts` scripts the AI conversation they show. They need Python 3 and ffmpeg with libx264 and libwebp. Start `pnpm dev`, then from the repository root:

```sh
pip install -r scripts/media/requirements.txt
playwright install chromium
python scripts/media/screenshots.py   # ai-assistant, ai-approval, sync, and sync-deploy PNGs
python scripts/media/video.py         # hatoba-demo.mp4 with music, and the silent hatoba-demo.webp for the README
```

`--out DIR` writes somewhere else for a look before replacing the files. The video's music is synthesized by `scripts/media/music.py`, so it carries no license.

## Checks before committing

CI also runs format and lint checks:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
pnpm lint     # ESLint for the desktop frontend; warnings fail too
```

## Troubleshooting

- When you add a dependency in `workers/sync`, npm 10 may fail with `Cannot read properties of null (reading 'edgesOut')`. This is a known npm 10.x issue with resolving the optional peer dependencies of vitest. Upgrade to npm 11 or use pnpm. A plain `npm install` or `npm ci` from the existing `package-lock.json` is not affected.
