# Settings

Open Settings with the gear in the sidebar header or with Ctrl+, (⌘, on macOS). It is a window over the app with seven tabs. Changes apply at once; there is no Save button. Esc or the close button closes it. Terminal settings are in `references/terminal.md` and AI settings in `references/ai-settings.md`.

## General

- **Right-Click in Terminal**: **Copy if selected, otherwise paste** (default, except on macOS) or **Show context menu** (default on macOS).
- **Confirm before pasting multiple lines**: on by default. Pasting text that contains line breaks shows a preview first.
- **Show host reachability**: on by default. Every 60 seconds Hatoba tries a TCP connection to the hosts in the list (no login) and colors the dot. A host with a proxy is tried through it, and counts as reachable once its SSH server answers.
- **Close Button** (Windows and Linux only): what the window's close button, Alt+F4 and the taskbar's Close do. **Ask every time** (default) asks whether to minimize Hatoba to the system tray or quit, and the question's **Remember my choice** sets this row. **Minimize to tray** hides the window while SSH sessions, port forwards and sync keep running; the tray icon brings it back. **Quit Hatoba** quits. Without a tray, on a Linux desktop with no StatusNotifier host such as GNOME without an extension, closing always quits. macOS has no such row: its red button hides the window, the Dock icon brings it back, and ⌘Q quits.
- **Notify when the AI assistant finishes or needs approval**: on by default. While Hatoba is not the active window, also while it is minimized or in the tray, a conversation that finishes a turn, stops on an error, or waits for approval of a tool call shows a system notification, also one in a tab's background. Its title is the tab's name (**AI Assistant** for the home tab), and its text says what happened and names the conversation. Clicking it brings the window back and shows the conversation with the AI panel open. The system's own notification settings for Hatoba, and Do Not Disturb, can still hide them.
- **Refresh interval** (3 s by default) and the three thresholds below it (CPU, memory and disk use, 90% by default, or **Off**) belong to the dashboard; see `references/dashboard.md`.
- **Export Encrypted Backup** → **Export…**: saves the vault as one file that only the master password opens. There is no import in the app.

## Appearance

- **Appearance**: **Light**, **Dark**, or **Auto** (follows the system and changes live).
- **Terminal Colors**: **Always Dark** (default), **Always Light** or **Match Appearance**, and a color scheme for a dark and a light terminal (Dracula, Nord, Solarized and others); see `references/terminal.md`.
- **List Density**: **Regular** or **Compact** rows in lists.
- **Language**: **System Default** (the system language if it is Chinese, Japanese or English, otherwise English), **简体中文**, **English** or **日本語**. It takes effect immediately. Host names, commands and terminal output are never translated.

## Proxies

SOCKS5 and HTTP proxies that hosts connect through. Choosing one for a host is in `references/hosts-and-connecting.md`.

- **Default proxy on this device**: **No proxy** or a saved proxy. Hosts whose proxy is **Device default**, and quick connections, use it. The choice stays on this device and does not sync, so a proxy that runs on this computer (such as Clash on 127.0.0.1) can be the default here and not on the others.
- **Saved Proxies** lists each proxy with its type, address and the number of hosts that name it; the device's default has a **Default** badge. Click a row to edit it; the trash icon deletes it after showing which hosts switch to the device default. Proxies sync with their usernames and passwords, encrypted like the rest of the vault.
- **Add Proxy** asks for **Name**, **Type** (SOCKS5 or HTTP; an HTTP proxy must support CONNECT), **Address** and **Port**, and a **Username** and **Password** when the proxy needs a sign-in. Pasting a proxy URL such as `socks5://127.0.0.1:7890` or `http://user:pass@proxy.example.com:3128` into **Address** fills in the other fields. A saved password shows **Saved** and can be replaced but not viewed. SOCKS5 and HTTP send the proxy's username and password to the proxy in the clear.

Other labels: **Edit Proxy**.

## Security

Auto-Lock (default 15 minutes, up to 8 hours or Never), **Lock when the computer sleeps** (on by default), **Disconnect all sessions when locked** (off by default), **Clear Clipboard** for copied host passwords (default 90 seconds), **Unlock with Windows Hello** (Windows) or **Unlock with Touch ID** (macOS), **Unlock with PIN** with **Require the master password after a restart** (on by default), the master password, the recovery code, and **Lock Now**. Details are in `references/keys-vault-and-lock.md`.

Other labels: **Master Password**, **Change…**, **Recovery Code**, **Generate New Code…**.

## About

- The page shows the version; click it to copy.
- **Software Update** → **Check for Updates** asks GitHub whether a newer version exists. The result is "Checking…", "Hatoba is up to date.", "Hatoba vX is available.", or an offline or failure message. A newer version's release notes show below the setting, with **View on GitHub** for its release page.
- **Download and Install** asks first only when SSH sessions are open or file transfers are running, and says how many closing Hatoba will end. After **Install and Restart** (or right away when nothing would end), Hatoba downloads the installer and checks its signature, then closes, which disconnects every session, and opens again once the installer finishes. An installer without a valid signature from Hatoba is refused and not run. Update error messages are in `references/troubleshooting.md`.
- **Automatically check for updates** is on by default. When on, Hatoba asks GitHub after the first unlock of each launch and then once a day while it keeps running, including hidden in the tray; when off, it contacts GitHub only when **Check for Updates** or **Download and Install** is pressed.
- When a newer release is found, the sidebar gear shows a dot, its tooltip becomes **Settings (update available)**, and it opens the About tab. On Windows and Linux the tray menu also shows **Update to vX.Y.Z**, which opens the About tab and starts **Download and Install**.
- **Website** opens https://hatoba.fog.moe, with downloads and these docs. **Source Code** and **MIT License** open the GitHub repository and the license. **Report a Problem** opens GitHub's bug report form with the Hatoba version and the operating system filled in; the user reviews and submits it.
- A card in the sidebar (**Enjoying Hatoba?**) asks once for a GitHub star, a day after first use and after a session has connected. **Star** opens the repository, **Report a Problem** opens the bug form, and the × closes it for good.

Other labels: **Copy version**.

## Which settings sync

| Synced with the vault | Kept on this device |
|---|---|
| Terminal settings (font, size, cursor, scrollback, terminal colors, right-click, multi-line paste check), the default AI model, the judge of Auto mode, the search provider, the built-in skill's switch, the AI custom instructions and Terminal Screen default, the memory switch, saved proxies | Language, appearance, list density, host reachability, the default proxy, the local terminal shell, the Security tab's Auto-Lock, sleep, disconnect and clipboard settings, the PIN, the update check switch, the AI permission mode, the saved AI approvals, the AI panel width and whether it is open, MCP on/off and Always allow |
