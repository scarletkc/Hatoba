# Contributing to Hatoba

Changes to Hatoba start as GitHub issues and land through pull requests. This
guide covers how to report a problem, name a branch, write commits, and open a
pull request. The [development guide](docs/development.md) covers building and
testing. The rules for taking issues and for pull request scope apply to
contributors without write access to the repository. Maintainers assign issues
and prepare releases.

## Report vulnerabilities privately

Do not open a public issue for a security vulnerability. Report it through
[private vulnerability reporting](https://github.com/scarletkc/Hatoba/security/advisories/new)
instead. This covers anything that exposes passwords, keys, or other vault data
beyond what the [security model](docs/hatoba-spec.md#4-security-model-and-encryption)
allows.

## Start with an issue

Open or find an issue before writing code. A pull request without an issue is
acceptable only for a small, self-evident fix, such as a typo or a broken link.
For anything larger, agree on the direction with a maintainer in the issue
before starting the work. Keep each pull request to one issue. When an issue
needs a large change, agree in the issue on how to split it into pull requests
that can each be reviewed on their own.

You can work on issues labeled `good first issue` or `help wanted` and on
issues you opened yourself. Comment on the issue when you start, so that nobody
else duplicates the work. Other issues often wait on a design decision or touch
the security model. For those, ask in the issue and wait until a maintainer
assigns it to you. A pull request for an issue that is neither open to you nor
assigned to you may be closed.

Search existing issues first. Title the issue with a one-line summary of the
problem rather than the solution, for example
`Terminal drops IME input after switching tabs`. Labels such as `bug`,
`enhancement`, and `documentation` classify the issue, so the title needs no
type prefix. Issues and pull requests can be written in English or Chinese.

### Describe the problem first

The reason for a change decides whether it belongs in Hatoba, so every issue
opens with it. For a feature request, describe:

- the task that fails or takes too long today, with a real example;
- where it happens: the platform, and whether sync is involved;
- what happens if nothing changes, and any workaround in use.

Put a proposed solution after the problem, if you have one. A request that
describes only a solution is sent back for its motivation before it is
considered.

### Include what the issue needs

- **Bug**: the Hatoba version or the commit you built from, the operating
  system and its version, the steps that reproduce the problem, what happened,
  and what you expected. Attach the relevant lines of the log; on Windows the
  logs are in `%LOCALAPPDATA%\app.hatoba.desktop\logs`.
- **Connection or authentication problem**: also the SSH server and its
  version, the authentication method, and whether the connection goes through
  ProxyJump or ssh-agent.
- **Sync problem**: also whether you sync through a Worker or directly to D1,
  and for a Worker, the `version` that its `/v1/health` endpoint reports.
- **Change to security, encryption, data formats, or the sync protocol**: the
  [architecture and requirements](docs/hatoba-spec.md) document defines how
  these must behave. Agree on the change in the issue first, and change the
  document in the same pull request as the code.

Logs leave out secrets and terminal content, but read them before attaching and
redact host details you do not want public. Never post a password, private key,
passphrase, recovery code, setup token, or session token.

## Name the branch

Branch names follow [Conventional Branch](https://conventionalbranch.org/):
`<type>/issue-<number>-<description>`, for example
`fix/issue-42-ime-after-tab-switch`. Leave out the `issue-<number>-` part only
for a change without an issue.

| Type | Use for |
| --- | --- |
| `feat/` | New features |
| `fix/` | Bug fixes |
| `hotfix/` | Urgent fixes to a published release |
| `release/` | Release preparation by maintainers, for example `release/v0.1.0` |
| `chore/` | Documentation, tests, CI, build, refactoring, and dependency updates |

Use lowercase letters, digits, and single hyphens. Dots are allowed only in
release versions.

Branch from the latest `main` and keep each branch to one change.

## Write commits

Commit messages follow
[Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/):

```text
<type>(<scope>): <description>

<body>

<footer>
```

| Type | Use for |
| --- | --- |
| `feat` | A new feature |
| `fix` | A bug fix |
| `perf` | A performance improvement without a behavior change |
| `refactor` | A code change that neither fixes a bug nor adds a feature |
| `docs` | Documentation only |
| `test` | Tests only |
| `build` | The installer, packaging, or dependencies |
| `ci` | GitHub Actions workflows |
| `style` | Formatting only |
| `chore` | Maintenance that fits no other type |
| `revert` | Reverting an earlier commit |

- The scope is optional. Use a lowercase crate or area name, such as `core`,
  `ssh`, `desktop`, or `worker`.
- Write the description in the imperative mood, without a trailing period,
  for example `fix(desktop): keep IME input when switching tabs`. Start it
  with a lowercase word; identifiers keep their case. Keep the header within
  72 characters.
- Use the body to explain what changed and why, wrapped at 72 characters.
- Mark a breaking change with `!` after the type or scope and a
  `BREAKING CHANGE:` footer. Users keep their vaults across versions and
  upgrade the app and their own Worker separately, so a change is breaking
  when existing data, an older app, or an older Worker cannot handle it. That
  covers the Worker API, the encrypted item format, the local and D1 schemas,
  and backup files. The Rust crate APIs and the Tauri commands between the
  frontend and Rust serve only the app, and changes to them are not breaking.
- Reference issues in the footer, for example `Refs #42`.

## Open a pull request

Title the pull request with a Conventional Commits header, as for a commit.
Pull requests are squash-merged, and the title becomes the commit on `main`.
The [PR title workflow](.github/workflows/pr-title.yml) checks the title and
runs again when you edit it.

The description gives a reviewer the context that the diff cannot:

- **Context**: the problem this solves and why it matters now, with a link to
  the issue. Summarize any decisions reached in the issue discussion.
- **Changes**: what changed, and the choices a reviewer might question.
- **Validation**: the commands you ran and their results, and the platforms
  you tested on.
- **Screenshots**: for UI changes, the screenshots that
  [Add screenshots for UI changes](#add-screenshots-for-ui-changes) requires.
- **Breaking changes**: what users must change, if anything, such as
  redeploying their Worker.

Link the issue with a closing keyword such as `Closes #42`, or with `Refs #42`
when the pull request addresses only part of it.

Before requesting review:

- Run the tests and checks in the [development guide](docs/development.md).
  CI runs on every update to the pull request, and every job must pass.
- Follow the [design notes](docs/design/README.md) for UI changes.
- When behavior changes, update the
  [architecture and requirements](docs/hatoba-spec.md) and any other affected
  docs in the same pull request; the
  [implementation status](docs/status.md) is the easiest to forget. Write
  documentation in English.
- Change a schema by adding a migration, never by editing an existing one:
  `MIGRATIONS` in `crates/hatoba-core/src/store.rs` for the local database,
  and `workers/sync/migrations/` for D1. Vaults and Workers that already ran a
  migration do not run it again.
- Update the branch with the latest `main`; only an up-to-date branch can merge.
- Change only what the issue asks for, and leave version numbers unchanged.
  Maintainers set them when preparing a release.

Open the pull request as a draft while the work is in progress. Merging
requires approval from a maintainer, and new commits dismiss an earlier
approval.

You are responsible for every line in your pull request, including lines an AI
tool wrote, so read the whole diff before requesting review. A pull request
that shows its author did not review it, such as one that edits files
unrelated to the issue, may be closed without a detailed review.

## Add screenshots for UI changes

**Every pull request that changes the UI must include screenshots in its
description or in a review comment, and is not ready to merge until they have
been checked visually. Passing tests do not replace them.** This covers changes
to layout, styling, components, visible text, and interaction states.

- Capture the running app with the pull request applied. Mockups and
  screenshots of an earlier implementation do not verify the change.
- For existing UI, show the same screen and state before and after the change.
  For a new screen or component, show the new UI and explain where it appears.
- Cover every affected screen and the states the change touches, such as
  empty, locked, offline, an open dialog, or a sync conflict.
- Include the light and dark themes and each interface language when the
  change can affect contrast, wrapping, or layout. When it affects layout, also
  include the minimum window size, set by `min_inner_size` in
  [`platform/window.rs`](apps/desktop/src-tauri/src/platform/window.rs).
- Label each screenshot with the screen or state, the platform, the window
  size, the theme, and the language.
- Check the screenshots for clipped text, overflow, overlap, misalignment, and
  controls that are hard to see or reach. Describe the interactions you checked
  separately; a screenshot cannot show that keyboard navigation or an action
  works.
- Refresh the screenshots when later commits change the UI they show.
- Show sample data only. Screenshots must not contain real host names,
  addresses, user names, key fingerprints, or anything else from your vault.

`pnpm dev` runs the frontend in a browser with the mock backend and the
design's sample data, and URL parameters select states and platforms, as the
[development guide](docs/development.md#run) describes. Capture changes that
depend on the native window or the operating system, such as the title bar,
Mica, or input methods, in the desktop app from `pnpm tauri dev` on the
affected platform.

If you cannot upload images in the browser and have push access to this
repository, `gh pr create` and `gh pr comment` attach them with `--attach`.
Check that `gh pr comment --help` lists the flag, and upgrade `gh` if it does
not. Put captions and notes on what you checked in `screenshots.md`, then
replace `PR_NUMBER` and the image paths:

```sh
gh pr comment PR_NUMBER --repo scarletkc/Hatoba --body-file screenshots.md --attach before.png --attach after.png
```

Without push access, upload the screenshots to an issue in your own public
fork and copy the image links into this pull request. Replace
`YOUR_USERNAME/Hatoba` with your fork, and `SCREENSHOT_ISSUE_NUMBER` with the
issue number that `gh issue create` prints:

```sh
gh repo edit YOUR_USERNAME/Hatoba --enable-issues
gh issue create --repo YOUR_USERNAME/Hatoba --title "Screenshots for Hatoba PR PR_NUMBER" --body-file screenshots.md --attach before.png --attach after.png
gh issue view SCREENSHOT_ISSUE_NUMBER --repo YOUR_USERNAME/Hatoba --json body --jq .body
```

Save the Markdown that `gh issue view` prints, which holds the uploaded image
links, as UTF-8 in `uploaded-screenshots.md`. Post it with
`gh pr comment PR_NUMBER --repo scarletkc/Hatoba --body-file uploaded-screenshots.md`
and check that reviewers can open the images.

Keep screenshots out of commits and the pull request's file changes, and do not
create a branch just to host them. If you cannot capture the affected UI,
explain why and keep the pull request in draft until the screenshots are added.

## License

Contributions are licensed under the project's [MIT License](LICENSE).
