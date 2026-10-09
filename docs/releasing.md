# Release Hatoba

A release publishes one commit on `main` as a GitHub Release with the Windows installer. The [Release workflow](../.github/workflows/release.yml) builds and signs the installer, waits for a maintainer to approve, and then tags the commit `vX.Y.Z` and publishes the release, from which installed apps update themselves. The scripts in [`scripts/release`](../scripts/release) prepare the version and run each step of the workflow. Run them from the repository root with Node.js 22 or later.

## Set up the release environment

Do this once. The workflow stops before building until the `release` environment requires a reviewer.

1. In the repository's **Settings → Environments**, create an environment named `release`.
2. Turn on **Required reviewers** and add the maintainers who approve releases.
3. Under **Deployment branches and tags**, choose **Selected branches and tags** and add `main`.

## Set up update signing

Installed apps install an update only when its installer is signed by the update signing key: they check the signature against the public key in `plugins.updater.pubkey` of [`tauri.conf.json`](../apps/desktop/src-tauri/tauri.conf.json), which every build carries. Do this once. The workflow stops before building while the public key is empty, or while the `release-signing` environment is missing or open to every branch.

1. Generate the key pair outside the repository, and enter a password when asked:

   ```sh
   pnpm tauri signer generate -w <a folder outside the repository>/hatoba-updater.key
   ```

2. Back up `hatoba-updater.key` and its password offline. GitHub never shows a secret again after you save it, so the backup is the only copy.
3. In the repository's **Settings → Environments**, create an environment named `release-signing`. Leave **Required reviewers** off: the build job that uses it runs before the approval in the `release` environment. Under **Deployment branches and tags**, choose **Selected branches and tags** and add `main`.
4. Add two environment secrets to `release-signing`: `TAURI_SIGNING_PRIVATE_KEY` with the contents of `hatoba-updater.key`, and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` with its password.
5. Set `plugins.updater.pubkey` in `tauri.conf.json` to the contents of `hatoba-updater.key.pub`, and merge that change to `main`.

### If the signing key is lost or exposed

Installed apps trust only the public key they were built with.

- **Lost key or password.** No later release can update the installed apps. Generate a new key pair, replace the secrets and the public key, and say in the next release note that users must download and run that installer by hand once. Later versions update as before.
- **Exposed key.** Whoever has the key and its password can sign installers that installed apps accept, if they can also get those apps to download them, for example by publishing a release in this repository. Replace the key as for a lost key, so that versions from the next release on reject installers signed with the exposed key.

## Prepare the version and the release note

The app and the crates share one version. `VERSION_FILES` in [`scripts/release/version.mjs`](../scripts/release/version.mjs) lists the files that record it, and a test in CI fails when they disagree. The sync Worker has a version of its own, which the pull request that changes the Worker raises ([Raise the Worker version](development.md#raise-the-worker-version)), so a release leaves it as it is.

On a `release/vX.Y.Z` branch from the latest `main`, run:

```sh
node scripts/release/bump.mjs minor --note "Release title"
```

The version argument is `patch` (the default), `minor`, `major`, or an explicit version such as `1.2.3` or `v1.3.0-rc.1`. A version is `MAJOR.MINOR.PATCH`, optionally followed by `-alpha.N`, `-beta.N`, or `-rc.N`, and the new version must be greater than the current one. From a prerelease, `patch` moves to its final release. Add `--dry-run` to list the files without changing them.

`--note` creates `docs/release-notes/X.Y.Z.md` with a `## Release title` heading. Below the heading, write what users get and what they must do, such as redeploying their Worker. A note is optional when an earlier release exists, but a note file must have both the heading and a body.

To release the version that the files already record, such as the first release, skip `bump.mjs` and create the note by hand. Without an earlier release there is no changelog, so the note is required.

Check the release and preview its notes:

```sh
node scripts/release/release.mjs check
node scripts/release/release.mjs notes --output target/release-notes.md
```

Commit the changes, open a pull request titled `chore(release): prepare vX.Y.Z`, and squash-merge it.

## Publish

1. In **Actions → Release → Run workflow**, choose `main`.
2. **Preflight** runs the script tests and checks that:
   - the version files agree, and `vX.Y.Z` does not point to another commit
   - the release note is valid, or an earlier release exists to list changes from
   - `plugins.updater.pubkey` in `tauri.conf.json` is a public key
   - CI passed on the commit, or on an earlier commit that differs from it only in files CI skips: Markdown files and `docs/`
   - the `release` environment requires a reviewer, and the `release-signing` environment allows only selected branches

   It then writes the release notes to the run summary.
3. **Build (Windows)** builds the installer and signs it with the update signing key, checks the signature against the public key in `tauri.conf.json`, and uploads the `hatoba-windows` artifact: `Hatoba_X.Y.Z_x64-setup.exe`, its signature `Hatoba_X.Y.Z_x64-setup.exe.sig`, `SHA256SUMS.txt`, and `latest.json`, which tells installed apps the version, the release notes, and where to download the installer.
4. **Publish** waits for approval. Read the notes in the run summary, and run the manual Windows checks in [§12 Testing](hatoba-spec.md#12-testing) with the installer from the artifact. Approve the deployment to publish, or reject it to stop without publishing.

Publishing tags the commit `vX.Y.Z` and creates the release **Hatoba vX.Y.Z** with those four files. Installed apps read `latest.json` from the latest release, so they find a stable release as soon as it is published. A prerelease is marked as a pre-release and never becomes the latest release, so installed apps never update to it.

## Release notes

`composeNotes` in [`scripts/release/release.mjs`](../scripts/release/release.mjs) builds the release notes from:

1. The note in `docs/release-notes/X.Y.Z.md`, when there is one.
2. Install instructions for the installer.
3. A changelog of the commits since the previous release, with a link to the full diff. Breaking changes, features, fixes, and performance improvements each have a section, sorted by the commits' Conventional Commits headers. The other commits are folded under **Other changes**.

`latest.json` carries the same notes without the install instructions, and **Settings → About** shows them with the update.

The previous release is the highest `vX.Y.Z` tag that the release commit contains and that is lower than the new version. A stable release skips prereleases, so its changelog covers the whole prerelease cycle, while a prerelease compares with any earlier release. For example, `1.3.0` compares with `v1.2.0` even when `v1.3.0-rc.1` exists, and `1.3.0-rc.2` compares with `v1.3.0-rc.1`.

## Retry a failed release

On the page of the failed run, choose **Re-run failed jobs**. The rerun uses the same commit and keeps the artifacts of the jobs that passed, and the **Publish** job waits for approval again. When the release already exists, the workflow keeps its notes and uploads only the files it is missing. It stops before publishing when:

- `vX.Y.Z` already points to another commit. Bump to a new version instead of moving the tag.
- A draft release uses the tag. Review the draft on GitHub, delete or publish it, and rerun the job.
- The release's pre-release mark does not match the version. Correct it on GitHub and rerun the job.
