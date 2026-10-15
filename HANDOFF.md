# Handoff: custom terminal color schemes (#98)

Work in progress, paused by the owner. Branch `scarletkc/cranky-archimedes-9738e1`, no PR yet. Delete this file in
the commit that finishes the work.

## Relation to PR 227

[PR 227](https://github.com/scarletkc/Hatoba/pull/227) (branch `scarletkc/preset-theme-colors-5db410`, open, not
merged) ships the 18 built-in schemes, the Dark / Light / Match Appearance choice and the synced `dark_scheme` /
`light_scheme` ids. This branch starts from its first commit, `31d2594`, and does not have its two later commits:

- `abc32f4` makes the scheme grid one tab stop with arrow keys. It changes `SchemePicker.tsx` and
  `SettingsWindow.test.tsx`, which this branch changes too, so expect conflicts there.
- `038f27d` describes the built-in schemes in the built-in skill (`references/terminal.md`, `references/settings.md`).

Before continuing, rebase onto PR 227's head, or onto `main` once it has merged (then the PR 227 commits drop out).

## Done

All three parts of the remaining scope have code and tests; nothing has been tried in the browser preview yet.

**Custom schemes**

- Core: a new item type `terminal_scheme` (`TerminalScheme`, `SchemeMode`, `SchemeColors` in
  `crates/hatoba-core/src/model.rs`), `Vault::terminal_schemes()`.
- Desktop commands `terminal_schemes_list`, `terminal_scheme_save`, `terminal_scheme_delete`
  (`apps/desktop/src-tauri/src/commands/schemes.rs`): name 1 to 64 characters, every color `#rrggbb` (stored in
  lowercase), field errors named after the color. The conflict view shows the name and all 20 colors.
- Frontend: `customScheme`, `paletteOf`, `toHex`, `modeOf` in `features/terminal/schemes.ts`; `schemeFor` and
  `schemesFor` take the custom list. `useTermSettings` loads the custom schemes with the settings, and
  `terminalOptions` / `resolveScheme` / the search colors use them.
- Settings → Appearance: each mode's grid lists the built-in schemes, then the user's. Under it, **Edit…** (custom
  schemes only; double-clicking a custom card does the same), **Duplicate…** and **Import…**.
- `SchemeEditor.tsx`: name, appearance (Dark / Light), a live preview, text / background / cursor / selection, and
  the 16 ANSI colors as normal and bright columns. Each color is a hex field with a swatch that opens the system
  color picker. Saving puts the scheme to use for its mode; **Delete** asks first.

**Import** (`features/terminal/schemeImport.ts`)

- Windows Terminal: one scheme object, a list, or a whole settings.json (comments and trailing commas allowed) with
  `schemes`; several schemes open a menu to pick one. `purple` / `brightPurple` are magenta.
- iTerm2 `.itermcolors`: XML plist, components read as sRGB; a file with only ` (Dark)` / ` (Light)` keys gives two
  schemes.
- Missing colors are filled: cursor from the text, selection from the text blended into the background, ANSI colors
  from Hatoba's scheme for the background's mode. Files over 1 MB, unreadable files and files without a scheme get
  an error toast.

**Line height and cursor blink**

- Settings → Terminal: a line height stepper (1.0 to 2.0 in steps of 0.1) after the font size, and a **Blinking
  Cursor** switch after the cursor style. The preview follows both. `terminalOptions` no longer hard-codes 1.3 and
  `false`.
- Synced as `terminal.line_height` (absent until changed, reads 1.3) and `terminal.cursor_blink` (absent while off).

**Strings and contract**: zh-CN, en and ja for every new string; `bindings.ts` regenerated; `types.ts`, `api.ts`,
`tauri.ts`, `contract.check.ts` and the browser mock updated.

## Design decisions

- **Storage.** Each custom scheme is its own `terminal_scheme` item, not a list inside the settings item. Builds that
  do not know an item type skip it on unlock and leave the row untouched in the store and on the server, whereas a
  list inside `settings` would be dropped whenever an older build saves its settings. Per-item rows also sync and
  resolve conflicts one scheme at a time.
- **Selection.** `dark_scheme` / `light_scheme` hold the scheme's item id (a UUIDv7, which `is_scheme_id` already
  accepts). An unknown id, including a deleted scheme's, reads as Hatoba Dark / Light, so deleting a scheme leaves
  the settings alone.
- **What older builds do.**
  - A build with PR 227 but without this branch keeps `dark_scheme` / `light_scheme` (it knows them), shows Hatoba's
    scheme for a custom id, and drops `line_height` / `cursor_blink` when it saves the settings.
  - A build before PR 227 also drops `dark_scheme` / `light_scheme` when it saves (already the case for PR 227).
  - Neither touches the `terminal_scheme` items. Covered by
    `sync::tests::terminal_schemes_outlast_settings_saved_by_an_older_build` and the existing
    `undecryptable_remote_items_are_skipped_not_fatal`.
- **Mode.** A scheme's `mode` is the list it shows in. Duplicating keeps the source's mode; importing uses the section
  the user clicked **Import…** in, whatever the background; the editor can change it. Light schemes get the contrast
  floor (3), dark ones none, as for the built-in schemes.
- **Missing colors.** A stored color that is empty or not `#rrggbb` (say, from a newer build) reads as the mode's
  default scheme's color. An unknown `mode` reads as dark.
- **Line height** is kept in hundredths in Rust (`LineHeight`) so settings stay `Eq`; the JSON holds the number of
  lines. A value outside 1.0 to 2.0 from a newer build reads as the nearest end; a non-number reads as unset.
- **Import parsing** runs in the frontend from an `<input type="file">`: no file-system access for the webview and no
  new Rust dependency. On macOS the input has no `accept`, because WKWebView greys out `.itermcolors`.

## Remaining work, in order

1. Rebase onto PR 227's head or `main` (see above) and resolve `SchemePicker.tsx` / `SettingsWindow.test.tsx`.
2. Spec (`docs/hatoba-spec.md`), not started:
   - §5.1: add `TerminalScheme` to the `Item` union and an interface for it (`type: "terminal_scheme"`, `name`,
     `mode`, `colors` with the 20 keys, `updated_at`); add `line_height?` and `cursor_blink?` to `Settings.terminal`;
     say that `dark_scheme` / `light_scheme` can name a `terminal_scheme` item; add a paragraph on what older builds
     keep (from "Design decisions").
   - TERM-06: custom schemes (duplicate, edit, import from Windows Terminal and iTerm2), line height, cursor blink.
   - §9 Settings row: mention the scheme editor and the import error states.
3. Built-in skill (`crates/hatoba-ai/src/skills/builtin/hatoba/references/terminal.md` and `settings.md`, which PR 227
   wrote for the built-in schemes): delete "There are no custom schemes and no import from other terminals." and
   describe **Edit…**, **Duplicate…**, **Import…**, the editor, line height and **Blinking Cursor**, with the UI
   strings in all three languages as those files list them.
4. Try it in the browser preview (`.claude/launch.json` config `desktop-web-alt`, port 1430): duplicate, edit with the
   color picker and hex fields, save, delete, import both formats, both modes with Match Appearance, the terminal
   taking the scheme after the settings window closes, line height and blink in a terminal. Check dark and light app
   themes and the editor's height in the 700×620 settings window.
5. Run the full checks (below), including `cargo clippy --workspace --all-targets -- -D warnings`, which has not been
   run on this branch.
6. Commit with a Conventional Commits message (squash merges use the PR title and body), open the PR, and close #98.

## Known issues and open questions

- Not tried in the preview or the desktop build; the layout of `SchemeEditor` (600 px sheet, two-column basic colors,
  ANSI table) is unchecked.
- Open terminals take scheme edits only when the settings window closes, as with PR 227's picks.
- The native color input is hidden from assistive tech and the tab order; the hex field is the keyboard path.
- `SettingsWindow.test.tsx` prints an "update … not wrapped in act(...)" warning; check whether the new tests cause it.
- The conflict view shows a scheme's colors as one string of 20 hex codes.
- `cargo clippy` not run; `cargo fmt` is clean.

## Verify

```bash
cargo test -p hatoba-core --lib
cargo test -p hatoba-desktop --lib
cargo test -p hatoba-desktop export_bindings
cargo clippy --workspace --all-targets -- -D warnings
pnpm --dir apps/desktop typecheck
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop test
```

Parallel Rust builds share `CARGO_TARGET_DIR=C:/Users/kc/Documents/GitHub/Hatoba/target/agent-shared`.

Last results on this branch, run filtered: the core `model::` tests and the new sync test, and the desktop
`schemes`, `settings`, `sync` and `export_bindings` tests pass. The full Rust suites have not been run. Frontend
typecheck and lint are clean and all 867 vitest tests pass (new: `schemeImport.test.ts`, more in `schemes.test.ts`
and `SettingsWindow.test.tsx`).
