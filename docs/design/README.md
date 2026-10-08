# Design

The visual and interaction reference is the Claude Design project `https://claude.ai/design/p/dbb8cd60-198e-4e17-9fe8-4e2dd4c7caf1`. [§0 of the architecture and requirements](../hatoba-spec.md#0-scope-and-conventions) says which one wins when the design and the spec disagree.

| File | Description |
|---|---|
| `Hatoba.html` | Single-file export of the design project that opens directly in a browser (fonts and icons included) |
| `source/Hatoba.dc.html` | Main design file: every page (light and dark) and the Chinese and English copy table `T` |
| `source/Sidebar.dc.html`, `source/TabBar.dc.html` | Sidebar and tab bar components used by the main file |
| `source/support.js` | Design canvas runtime (it only renders the design, and the app does not use it) |

## Porting conventions

- Colors, radii, and sizes are design tokens in `apps/desktop/src/styles/tokens.css`. Light values come from the variables on the design's root node and dark values from the `DARK` table in its script. Components reference only the variables.
- Icons match the design and use Phosphor (`@phosphor-icons/web`, bundled locally with no remote resources).
- Copy lives in `apps/desktop/src/i18n/locales/`. Chinese and English follow the design's `T` table, and Japanese is added on top.
- [Spec §9.1](../hatoba-spec.md#91-windows-adaptation) covers the platform replacements on Windows (title bar, window buttons, fonts). In copy, "Touch ID" becomes "Windows Hello", and "this Mac" (本 Mac) becomes "this device" (本机).

## Deviations from the design

The implementation differs from the design in these places, all following the spec:

- Encryption description: the design says XChaCha20-Poly1305, but the implementation uses AES-256-GCM and Argon2id per spec §4, and the copy is corrected to match.
- Worker connection: the design's access token (`SYNC_TOKEN`) is replaced by the setup token (`SETUP_TOKEN`) from spec §6.2, which is used only for the first initialization.
- Sync conflicts: the design pauses sync and asks for a choice on each item. Spec §6.4 requires automatic resolution with a log, so the conflict page instead lists the automatically resolved conflicts for review, where each one can keep the current result or restore the other version.
- Recovery code: shown as 8 groups of 4 characters, per the format in spec §4.1.
- The unlock page footer does not show the host count: item types must not be stored in plaintext (spec §4.2), so the count is unknown while the vault is locked.
- Viewing a private key (KEY-07) is P2 and would hand the private key to the WebView, so the MVP has no such button.
