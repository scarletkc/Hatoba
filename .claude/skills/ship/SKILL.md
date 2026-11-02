---
name: ship
description: How the owner's changes to scarletkc/Hatoba get from a branch to main, and releases out. Use when committing, opening a pull request, answering review bots, merging, or releasing in this repository.
---

# Ship a change

This is the owner's flow. Outside contributors follow `CONTRIBUTING.md` instead.

## Branch

- Several sessions work in the main checkout at once. Work in a worktree of your own, branched
  from `origin/main`. In the main checkout, never switch branches, stash, or revert files you did
  not edit.
- One change is one pull request, however many areas it touches. Split only changes that are
  genuinely different. On long work, commit a checkpoint whenever a milestone is done.
- Agents that build Rust at the same time share one `CARGO_TARGET_DIR`. A target directory per
  agent fills the disk.

## Check

Run the tests that cover the change, not whole suites: CI runs everything. Run Prettier on the
files you changed, because CI's Frontend job runs `pnpm format:check`.

## Pull request

- The title is a Conventional Commits header, which the PR title check requires. It becomes the
  subject of the squash commit on `main`.
- The body is a few sentences on what changed and why, because it becomes the squash commit's
  body word for word. Skip the pull request template and its checklist.
- Commits and pull requests carry no AI attribution. A squash merge copies any `Co-authored-by`
  trailer from the branch's commits, so keep them out of those too.

## Review bots

CodeRabbit, Devin Review and Codex review each pull request. CI decides the merge, not the bots.

- Fix what a real user or maintainer plausibly runs into.
- Decline the rest: reply in the thread with the reason, then resolve it.
- After a round of fixes, ask for one more `@codex review` at most.
- Comments you post on GitHub are the owner's words: first person, no AI footer. A problem that
  belongs to another open pull request of the owner's goes there as a comment.
- Moving code reopens a CodeQL alert that was dismissed at the old path. Ask the owner to
  dismiss the new one with the old reason.

## Merge

Merge only when the owner asks.

1. Wait for CI. A pull request that changes only Markdown can merge right away.
2. Read the pull request's comments (`gh pr view <n> --comments`): the owner posts reviews there.
3. Re-read the body right before merging. Remove the `devin-review-badge` block that Devin Review
   appends, and anything else that does not belong in the commit message.
4. Run `gh pr merge --repo scarletkc/Hatoba <n> --squash --admin`. The ruleset requires an approval
   that the owner's own pull requests never get, so auto-merge would wait forever. Keep `--repo`
   right after `merge`: the permission rule that allows the command matches that prefix. GitHub
   deletes the head branch.
5. Remove your worktree: `git worktree remove <path>`. On Windows it leaves the `node_modules`
   links behind, so delete the rest with `Remove-Item -LiteralPath <path> -Recurse -Force` and run
   `git worktree prune`. Then delete the local branch with `git branch -D <branch>`.

## Release

`docs/releasing.md` has the steps. Merging the release pull request starts the Release workflow,
and publishing waits for the owner's approval.
