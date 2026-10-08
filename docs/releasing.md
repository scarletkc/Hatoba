# Release Hatoba

A release publishes one commit on `main` as a GitHub Release with the Windows installer. The [Release workflow](../.github/workflows/release.yml) builds the installer, waits for a maintainer to approve, and then tags the commit `vX.Y.Z` and publishes the release. The scripts in [`scripts/release`](../scripts/release) prepare the version and run each step of the workflow. Run them from the repository root with Node.js 22 or later.

## Set up the release environment

Do this once. The workflow stops before building until the `release` environment requires a reviewer.

1. In the repository's **Settings → Environments**, create an environment named `release`.
2. Turn on **Required reviewers** and add the maintainers who approve releases.
3. Under **Deployment branches and tags**, choose **Selected branches and tags** and add `main`.

## Prepare the version and the release note

The app, the crates, and the sync Worker share one version. `VERSION_FILES` in [`scripts/release/version.mjs`](../scripts/release/version.mjs) lists the files that record it, and a test in CI fails when they disagree.

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
   - CI passed on the commit, or on an earlier commit that differs from it only in files CI skips: Markdown files and `docs/`
   - the `release` environment requires a reviewer

   It then writes the release notes to the run summary.
3. **Build (Windows)** builds the installer and uploads `Hatoba_X.Y.Z_x64-setup.exe` and `SHA256SUMS.txt` as the `hatoba-windows` artifact.
4. **Publish** waits for approval. Read the notes in the run summary, and run the manual Windows checks in [§12 Testing](hatoba-spec.md#12-testing) with the installer from the artifact. Approve the deployment to publish, or reject it to stop without publishing.

Publishing tags the commit `vX.Y.Z` and creates the release **Hatoba vX.Y.Z** with the installer and `SHA256SUMS.txt`. A prerelease is marked as a pre-release and never becomes the latest release.

## Release notes

`composeNotes` in [`scripts/release/release.mjs`](../scripts/release/release.mjs) builds the release notes from:

1. The note in `docs/release-notes/X.Y.Z.md`, when there is one.
2. Install instructions for the installer.
3. A changelog of the commits since the previous release, with a link to the full diff. Breaking changes, features, fixes, and performance improvements each have a section, sorted by the commits' Conventional Commits headers. The other commits are folded under **Other changes**.

The previous release is the highest `vX.Y.Z` tag that the release commit contains and that is lower than the new version. A stable release skips prereleases, so its changelog covers the whole prerelease cycle, while a prerelease compares with any earlier release. For example, `1.3.0` compares with `v1.2.0` even when `v1.3.0-rc.1` exists, and `1.3.0-rc.2` compares with `v1.3.0-rc.1`.

## Retry a failed release

On the page of the failed run, choose **Re-run failed jobs**. The rerun uses the same commit and keeps the artifacts of the jobs that passed, and the **Publish** job waits for approval again. When the release already exists, the workflow keeps its notes and uploads only the files it is missing. It stops before publishing when:

- `vX.Y.Z` already points to another commit. Bump to a new version instead of moving the tag.
- A draft release uses the tag. Review the draft on GitHub, delete or publish it, and rerun the job.
- The release's pre-release mark does not match the version. Correct it on GitHub and rerun the job.
