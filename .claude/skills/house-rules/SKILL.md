---
name: house-rules
description: The owner's calls for scarletkc/Hatoba that the repository doesn't spell out, on UI copy, translations, docs, the assistant's prompts and UI, plus the commit, pull request and merge flow. Use before writing or reviewing any of these, or shipping a change.
---

# House rules

## Copy and UI

### Judge like this

- **Do what comparable apps do, then decide yourself.** The apps are listed in
  [principle 5 of spec §3.2](../../../docs/hatoba-spec.md#32-core-principles), which also limits the
  text shown up front. What modern apps ship beats older platform guidelines: buttons and menu items
  have no trailing "…" (#316) although the macOS and Windows guidelines ask for one. Only progress
  text and placeholders keep it. Conveniences survive the cuts: an MCP preset only fills in fields,
  but its "create token" link stays (#116).
- **Design for someone opening Hatoba for the first time.** Add no "moved here" hint, migration note
  or note for existing users.
- **Every sentence holds in every state, mode and platform.** *Counter-example: a Touch ID error told
  the user to use their master password, which is wrong for a vault unlocked with a PIN (#217).*
- **Leave out what users can't see.** Release notes and posts don't mention prompt rewrites or
  refactors.

### UI copy

- English uses Title Case for buttons, menu items, tabs, section headers, setting labels that are
  noun phrases, and the titles of dialogs that do something. It uses sentence case for switch
  labels, choices, tooltips, status lines, empty states and titles that are questions. The website
  uses sentence case for its buttons and links. No em dash, exclamation mark, "please" or "we".
- Labels say what happens, and don't read like their neighbors. "Auto-start" next to a Start button
  became "Start on connect" (#284), and "Leave Out of Dashboard" became "Don’t Read Usage" (#306).
- Hints show only when they apply, and never refer to something the reader hasn't seen (#234).
- Delete confirmations don't explain sync: "This can’t be undone." is enough.
- The UI implies nothing the user didn't do. The permission mode divider shows only when the mode
  changes, because on the first message it read like a switch nobody made (#160).

### Translations

- zh-TW is written for Taiwan, not converted: 「」 quotes and Taiwan terms (連線, 終端機, 連接埠轉送,
  工作階段). OpenCC gives a first draft at most (#329).
- Developer-facing errors stay in English, as other tools leave them: `AppError.detail` and
  messages from MCP servers. Translate a whole family of errors or none of it.
- Every language looks the same: the same sizes, weights and line heights, and no line break inside
  a CJK word.

### The assistant's prompt and tool descriptions

- The prompt holds the context, how to choose tools, and rules that span tools. A tool description
  holds what the tool does, its limits with exact numbers, its preconditions, and what to do when it
  fails. Lists of allowed values go in the schema. Nothing is said in both places (#105, #242).
- Every statement holds in every permission mode, and the prompt names no value that a user setting
  decides: the current value arrives as a note in the messages (#160).
- No IMPORTANT, MUST or capitals for emphasis, no extra line that restates a rule more firmly, and no
  prompt variants per model.
- Explain only what is active. The prompt explained `<mcp_tools>` even when no tools were deferred,
  and models told users that tools were missing (#336).
- Names and parameters mean what a model expects: `read_terminal` with `lines: 5` returns the last
  five lines. One concept gets one word: memory notes and the `memory` tool both say `scope` (#276).
- A prompt change starts from a real model mistake, a false statement or a repeated fact. A
  reviewer's "this could be spoofed" is not a reason to add text.

### Docs

- The built-in `hatoba` skill names no outside tools, and no alternatives to what the app shows.
- The README's only media is the promo video. No screenshots, no translated README, no links to
  policy files.
- Positioning copy (the tagline, the README intro, the website's headings, the repository
  description) promises nothing about how the AI is provided: no "bring your own key", no "Hatoba
  runs no AI service".
- Docs record no verification status, such as "verified on a real Mac" or "not yet tested on
  Linux". Ask the owner what has been checked.

### Look and behavior

- Nothing changes size when its state changes. A button doesn't carry status: progress goes in the
  status line (#284, #285, #305).
- Reuse a pattern the app already has before making a new one: the keys list works like the hosts
  list (#390), and key sequences are edited as chips like host tags (#234).
- AI surfaces follow Claude Code and Codex: the model picker on the right of the composer, the
  thinking level always shown, a context ring colored rather than labeled with a percentage.
- Follow the platform: no tray icon on macOS, menus that behave like native ones, and Ctrl with the
  mouse wheel changes the terminal's font size, not the window's.

### Decide or ask

Decide yourself, and say what you chose in one line of the pull request or reply: all wording in
every language, including strings the owner wrote earlier; layout and interaction details; which of
two reasonable options to take. Don't hand these back as a list of uncertain spots, screenshots for
the owner to judge, or a menu of options. When the owner asks "is this reasonable?" or "what do
others do?", check comparable apps and come back with a decision.

Ask the owner first about positioning copy and anything published under the owner's name, about
departing from what comparable apps do in either direction, and about product scope: adding or
removing a settings page, a feature, or the way into one.

External contributors get no work on copy, translations, docs, prompts or how the UI looks. When
their pull request still gets these wrong after a review, keep their logic and rewrite everything a
user or the model reads in an owner pull request, with `Co-authored-by: <contributor>` in its body
(#234 became #242).

## Shipping

### Commits and pull requests

- On long work, commit a checkpoint whenever a milestone is done.
- The pull request body is a few sentences on what changed and why, because it becomes the squash
  commit's body word for word. Skip the pull request template and its checklist.
- A squash merge copies `Co-authored-by` trailers from the branch's commits into the commit on
  `main`, so branch commits carry no AI co-author trailer.

### Review bots

CodeRabbit, Devin Review and Codex review each pull request. CI decides the merge, not the bots.

- Fix what a real user or maintainer plausibly runs into.
- Decline the rest: reply in the thread with the reason, then resolve it.
- After a round of fixes, ask for one more `@codex review` at most.
- Comments you post on GitHub are the owner's words: first person, no AI footer. A problem that
  belongs to another open pull request of the owner's goes there as a comment.
- Moving code reopens a CodeQL alert that was dismissed at the old path. Ask the owner to dismiss the
  new one with the old reason.

### Merge

1. Wait for CI. A pull request that changes only Markdown can merge right away.
2. Read the pull request's comments (`gh pr view <n> --comments`): the owner posts reviews there.
3. Run `gh pr merge --repo scarletkc/Hatoba <n> --squash --admin`. The ruleset requires an approval
   that the owner's own pull requests never get, so auto-merge would wait forever.
4. If you made a worktree yourself with `git worktree add`, remove it with `git worktree remove
   <path>`. On Windows it leaves the `node_modules` links behind, so delete the rest with
   `Remove-Item -LiteralPath <path> -Recurse -Force` and run `git worktree prune`. Then delete the
   local branch with `git branch -D <branch>`. Leave the worktree the app made for your session, the
   one you run in: archiving the session cleans it up.
