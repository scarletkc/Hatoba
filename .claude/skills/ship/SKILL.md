---
name: ship
description: How the owner's changes to scarletkc/Hatoba get from a branch to main. Use when committing, opening a pull request, answering review bots, or merging in this repository.
---

# Ship a change

## Commits and pull requests

- On long work, commit a checkpoint whenever a milestone is done.
- The pull request body is a few sentences on what changed and why, because it becomes the squash
  commit's body word for word. Skip the pull request template and its checklist.
- A squash merge copies `Co-authored-by` trailers from the branch's commits into the commit on
  `main`, so commits carry none.

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

1. Wait for CI. A pull request that changes only Markdown can merge right away.
2. Read the pull request's comments (`gh pr view <n> --comments`): the owner posts reviews there.
3. Run `gh pr merge --repo scarletkc/Hatoba <n> --squash --admin`. The ruleset requires an approval
   that the owner's own pull requests never get, so auto-merge would wait forever.
4. If you worked in a worktree, remove it with `git worktree remove <path>`. On Windows it leaves
   the `node_modules` links behind, so delete the rest with
   `Remove-Item -LiteralPath <path> -Recurse -Force` and run `git worktree prune`. Then delete the
   local branch with `git branch -D <branch>`.
