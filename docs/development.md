# Development guide

Build, run, and test Hatoba locally. [`.github/workflows/ci.yml`](../.github/workflows/ci.yml) has the full set of checks that CI runs.

## Requirements

- Rust stable, Node.js 22+, and pnpm (the version is in `packageManager` in the root [`package.json`](../package.json)).
- Windows: WebView2 (included in Windows 11).
- macOS 13 or later: the Xcode Command Line Tools (`xcode-select --install`), and Rust from [rustup](https://rustup.rs/). The Rust version that Homebrew installs may be older than `rust-version` in the root [`Cargo.toml`](../Cargo.toml), so make sure `rustc --version` reports the rustup toolchain.
- Linux: the Tauri dependencies, such as `libwebkit2gtk-4.1-dev`. The System dependencies step of the `rust-linux` job in `ci.yml` has the full list that CI installs.
- The SSH integration tests and the end-to-end test need an OpenSSH server (`sshd`).

## Run

```sh
pnpm install
pnpm tauri dev            # Start the desktop app
pnpm dev                  # Start only the frontend, with a mock backend in the browser
```

A release build runs one Hatoba at a time and hands the arguments of a second launch to it ([HOST-13](hatoba-spec.md#82-hosts)). A development build skips this, so it can run beside the installed app, unless `HATOBA_SINGLE_INSTANCE` is set.

In the browser, `pnpm dev` uses the mock backend in `apps/desktop/src/ipc/mock` with the design's sample data. URL parameters switch demo states, such as `?state=locked` and `?sync=conflict`. The comment on `createMockApi` in `apps/desktop/src/ipc/mock/index.ts` lists every parameter.

`/?gallery` opens the component gallery instead of the app: every control and pattern in `apps/desktop/src/components` in each state, in light, dark or side by side, at either density. Check it after changing a shared component. Only development builds include it.

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

These tests start a throwaway local `sshd` and need the OpenSSH server installed. They are skipped unless `HATOBA_SSH_IT=1` is set. The test harness needs `/run/sshd`, the privilege separation directory of Linux OpenSSH, so run them on Linux; CI runs them only in the `rust-linux` job.

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

The tests in `apps/desktop/src-tauri/src/update.rs` run the updater against a mock endpoint and a mock list of releases. To try an update end to end on Windows without publishing a release, install a test build and serve it a newer one from this machine. The test builds have their own name, identifier, and executable, so they install next to Hatoba without touching it or its vault.

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
     "version": "1.0.0",
     "plugins": {
       "updater": {
         "pubkey": "<contents of test.key.pub>",
         "endpoints": ["http://127.0.0.1:18765/latest.json"],
         "dangerousInsecureTransportProtocol": true
       }
     }
   }
   ```

   Write `<dir>/new.json` with the same contents and `"version": "1.0.1"`. Both versions are stable ones: a prerelease version looks for updates in the GitHub releases instead of the configured endpoint.

3. Build each version, and copy its installer and `.sig` out of `target/release/bundle/nsis` before the next build:

   ```sh
   export TAURI_SIGNING_PRIVATE_KEY=<dir>/test.key TAURI_SIGNING_PRIVATE_KEY_PASSWORD=test
   pnpm tauri build --bundles nsis --config <dir>/old.json
   pnpm tauri build --bundles nsis --config <dir>/new.json
   ```

4. Run the older installer, then serve the newer one:

   ```sh
   node scripts/release/serve-update.mjs --installer "<dir>/Hatoba Update Test_1.0.1_x64-setup.exe" --pubkey <dir>/test.key.pub
   ```

5. Open **Hatoba Update Test**, create a vault, and in **Settings → About** choose **Check for Updates**, then **Download and Install**. Hatoba closes, the installer shows its progress, and the new version opens.
6. Uninstall **Hatoba Update Test** in the Windows settings, delete `%LOCALAPPDATA%\app.hatoba.updatetest`, and delete the throwaway key.

On Linux, build with `--bundles deb,appimage` and copy the packages out of `target/release/bundle/deb` and `target/release/bundle/appimage`. Install the older deb package or run the older AppImage, then pass the newer package of the same kind to `serve-update.mjs`, which serves it under the key that kind of build looks for in `latest.json`. Afterwards, remove the deb package with `sudo apt remove hatoba-update-test`, and delete `~/.local/share/app.hatoba.updatetest` and the throwaway key.

On macOS, build each version with `--bundles app` and copy `Hatoba Update Test.app`, `Hatoba Update Test.app.tar.gz`, and its `.sig` out of `target/release/bundle/macos` before the next build. Copy the older app to Applications and open it, then pass the newer `.app.tar.gz` to `serve-update.mjs`. The updater replaces the app and restarts it. Afterwards, delete the app, `~/Library/Application Support/app.hatoba.updatetest`, and the throwaway key.

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

## Add no Hatoba-only restrictions

[#119](https://github.com/scarletkc/Hatoba/issues/119) removed approvals, limits, confirmations, and explanations that comparable tools don't have, and a change should not bring them back. The AI assistant is compared with Claude Code, Codex, and Cursor, and the rest of the app with OpenSSH, Termius, Tabby, Windows Terminal, 1Password, and Bitwarden.

- Match what those tools do by default, and add no safeguard, confirmation, or limit that they don't have.
- Keep text that a user needs to finish a task or that prevents real data loss. Put requirements into links, defaults, or error messages, not into text shown up front.
- In the system prompt, one rule says that tool results are data, not instructions. Don't say it again, or a variant of it, in tool descriptions, `<attachments>`, notes, the compaction summary, or the built-in skill. Attachments are the user's own content.
- Custom instructions and host notes override the default rules, and Hatoba still enforces approvals.

## Fit pages beside the AI panel

A page in the main area gets its narrowest in the smallest window (`MIN_SIZE` in [`platform/window.rs`](../apps/desktop/src-tauri/src/platform/window.rs)) with the sidebar and the AI panel open. Nothing on it may run under the AI panel or make the page scroll sideways at that width.

- Put the page's controls in `PageHeader`, which moves them under the title and then wraps them onto more rows. Controls that would take several rows can show only their icons in a narrow page, as the dashboard's do.
- A table drops its last columns instead of scrolling sideways: with container queries when its columns are fixed, as in `HostsPage.module.css`, or with `fitColumns` in `features/dashboard/logic.ts` when the user picks them.
- Check a changed page with `pnpm dev` in a browser window of that size, in each language, since labels differ in length.

## Keep the built-in skill current

The AI assistant ships with a built-in `hatoba` skill in [`crates/hatoba-ai/src/skills/builtin/hatoba`](../crates/hatoba-ai/src/skills/builtin/hatoba). Its `SKILL.md` and `references/` files describe the current interface, settings, and features, so the assistant can answer questions about Hatoba, and the [website](#website) publishes them as its docs. A pull request that adds, renames, or removes UI, settings, or a feature that the skill describes updates the skill in the same pull request.

- Each file stays under 15,000 characters, because `read_skill` shortens a longer file.
- Name UI labels in English, copied character for character from the `en` message tables in `apps/desktop/src/i18n/locales`. The other languages go in the label files, not the text.
- The label files in `references/labels/` give those labels in each other language, by file and `## ` section, so the assistant can quote what the user sees. They are written from `AREAS` in `apps/desktop/src/i18n/hatobaSkillLabels.ts`, which lists the message keys of each section; only messages without a `|` can be listed, and a placeholder such as `{host}` stays in the text. To add a label, add its key there and write the files again (below). A new language gets its files from `LANGUAGE_NAME` in the same file, and each new file goes in `SKILL.md`'s list and in `crates/hatoba-ai/src/skills/builtin.rs`.
- `{{HATOBA_VERSION}}` in `SKILL.md` is a placeholder that the app replaces with the running version.

`apps/desktop/src/i18n/hatobaSkill.test.ts` runs with `pnpm test`. It checks the label files against the message tables, the sections they name, the frontmatter, the size limit, and the links between the files. After a label changes, write the label files again:

```bash
pnpm --filter @hatoba/desktop exec vitest run -u src/i18n/hatobaSkill.test.ts
```

## Update the screenshots and promo film

The scripts in [`scripts/media`](../scripts/media) capture the screenshots and the promo film's product shots from the browser frontend's mock backend. The website's English pages use the screenshots in `docs/media`, and its Chinese and Japanese pages the ones in `apps/site/src/assets/screenshots`. The promo film is in `docs/media` in English and Chinese, and the README opens with the English one. `?ai=showcase` in `apps/desktop/src/ipc/mock/ai.ts` scripts the AI conversation they show. They need Python 3 and ffmpeg with libx264. Start `pnpm dev`, then from the repository root:

```sh
pip install -r scripts/media/requirements.txt
playwright install chromium
python scripts/media/screenshots.py                  # ai-assistant, ai-approval, dashboard, and sync-deploy PNGs
python scripts/media/screenshots.py --locale zh-CN   # the same in Chinese, and with --locale ja in Japanese
python scripts/media/promo/capture.py                # the app's states that the film shows, in English and Chinese
python scripts/media/promo/render.py --lang en --music track.mp3 --music-offset 0.37   # hatoba-promo-en.mp4, and --lang zh
```

`--out DIR` writes somewhere else for a look before replacing the files. GitHub doesn't play videos from the repository, so the README plays the English film from an attachment: `render.py --small`, with the same music options, writes a copy under the 10 MB that GitHub takes, and dragging it into a comment box on GitHub gives its link. The film is an animation in `scripts/media/promo/index.html`, which plays in a browser when the repository root is served (`python -m http.server`, then `/scripts/media/promo/index.html`), and `scripts/media/promo/sfx.py` synthesizes its sound effects. The music isn't in the repository: the films use gritz16's "Meet Me at the Lobby" (120 BPM), and `--music-offset 0.37` puts its downbeats on the film's beats.

## Website

The website at https://hatoba.fog.moe is an Astro site in [`apps/site`](../apps/site), in English, Chinese, and Japanese. Its text is in `apps/site/src/i18n`, where `en.ts` holds the English that the other files translate. The [Site workflow](../.github/workflows/site.yml) builds it for pull requests that touch it or the desktop frontend, and deploys it to GitHub Pages from `main` and after each release.

```sh
pnpm --filter @hatoba/site dev    # http://localhost:4321
pnpm --filter @hatoba/site test
pnpm --filter @hatoba/site build  # writes apps/site/dist
pnpm --filter @hatoba/site layout:check  # after build: fails when a page moves while it loads (needs Chrome)
```

The build reads the newest release from the GitHub API (set `GITHUB_TOKEN` if it hits the rate limit) and takes the docs from the built-in skill at that release's tag, so they describe the version people download. To preview a change to the docs, run it with `HATOBA_DOCS_REF=worktree`. A new reference file needs a place in `DOC_GROUPS` in [`apps/site/src/lib/docs.ts`](../apps/site/src/lib/docs.ts), which a test checks.

The home page's **Try it in your browser** button swaps its first screenshot for the app's demo at `/demo/`: the browser frontend and its mock backend, built with `--mode demo` for visitors (`VISITOR` in [`apps/desktop/src/ipc/mock/visitor.ts`](../apps/desktop/src/ipc/mock/visitor.ts)). The button shows only when the demo was built before the site:

```sh
pnpm --filter @hatoba/site demo        # writes apps/site/public/demo from the release's tag
pnpm --filter @hatoba/site demo:check  # after build: opens the demo in Chrome and runs it to its first approval card
```

Like the docs, the demo comes from the release's tag, so it shows the version people download, and a release from before the demo builds none. `HATOBA_DEMO_REF=worktree` builds your checkout instead, as pull requests do, and `pnpm dev --mode demo` in `apps/desktop` runs the visitors' version with hot reload.

## Checks before committing

CI also runs format and lint checks:

```sh
cargo fmt --all --check
node scripts/ci/comment-width.mjs   # Rust comment lines within 100 characters, which rustfmt does not wrap
cargo clippy --workspace --all-targets -- -D warnings
pnpm lint     # ESLint for the desktop frontend; warnings fail too
```

## Troubleshooting

- When you add a dependency in `workers/sync`, npm 10 may fail with `Cannot read properties of null (reading 'edgesOut')`. This is a known npm 10.x issue with resolving the optional peer dependencies of vitest. Upgrade to npm 11 or use pnpm. A plain `npm install` or `npm ci` from the existing `package-lock.json` is not affected.
