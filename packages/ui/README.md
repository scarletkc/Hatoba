# @hatoba/ui

Hatoba's components and design tokens: buttons, fields, menus, dialogs, sheets, toasts and list
patterns, in light and dark. The desktop app uses them, and so will the apps that follow it.
The package is private to this workspace and is not published.

## Use

Depend on it from another package in the workspace, and load its base styles once:

```json
"dependencies": { "@hatoba/ui": "workspace:*" }
```

```css
@import "@hatoba/ui/base.css";
```

`base.css` brings the Phosphor icon fonts and the tokens. To read only the variables, import
`@hatoba/ui/tokens.css`.

The components speak English. Pass them the app's language through `UiStringsProvider`, as
`apps/desktop/src/app/UiStrings.tsx` does. Mount `ConfirmHost` and `ToastHost` once, for
`confirm()` and `toast()`.

## Attributes on the root

The app sets these on `<html>`, and the tokens follow:

- `data-theme="dark"`: the dark palette.
- `data-density="compact"`: shorter list rows.
- `data-platform="macos"`: the native macOS fonts.
- `data-theme-switch`, while the theme changes: no fades from the old colors.
- `data-floating`, with `--win-pad` and `--win-pad-px`, for a window that paints its own frame
  around a transparent margin: overlays stay inside the frame.

`setOverlaysLocked(true)` hides dialogs and closes menus behind a lock screen.

## Develop

The desktop app's development build shows every component in each state at `/?gallery`
(`apps/desktop/src/app/Gallery.tsx`). From the repository root:

```sh
pnpm --filter @hatoba/ui typecheck
pnpm --filter @hatoba/ui lint
pnpm --filter @hatoba/ui test
```
