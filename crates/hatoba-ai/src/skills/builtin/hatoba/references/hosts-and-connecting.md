# Hosts and connecting

## The host list

The home tab shows the host list for the sidebar item that is selected: **All Hosts**, **Favorites**, **Recent** (hosts that have been connected before), a group, or a tag. A host is one saved SSH destination. Groups are flat (one level) and a host belongs to at most one group; tags are free text and a host can have several.

- The header has a search box (fuzzy match on name, address, user name and tags; Ctrl+Shift+K or ⌘⇧K focuses it), the sort button, and **New Host**. The search box also connects to hosts that are not saved; see Quick connect below.
- Each row shows the name, `user@address:port`, the jump host if any, tags, and the time of the last connection. The dot is green when the host is connected or answers a TCP probe, grey otherwise. A host whose latest attempt in this launch failed shows **Failed** in red.
- To connect: double-click a row, select it and press Enter, or click **Connect**. Up and down move the selection, and typing starts a search. The same host can be opened in several tabs.
- Right-click a row for its menu.

Other labels: **Search hosts or ssh user@host**, **Sort by**, **Name**, **Address**, **Tags**, **Last Connected**, **Duplicate**, **Add to Favorites**, **Remove from Favorites**, **Copy Address**.

- **Copy Password** (hosts with a saved password) clears the clipboard after the delay in Settings → Security.
- **Delete…** asks for confirmation. With Cloud Sync on, the host is removed on the other devices too.
- A group's right-click menu in the sidebar has **Rename Group** and **Delete Group**. Deleting a group moves its hosts to All Hosts; it does not delete them.
- The reachability dots can be turned off in Settings → General (**Show host reachability**). When on, Hatoba tries a TCP connection to the listed hosts every 60 seconds, with no login attempt.

## Quick connect

Typing `user@host[:port]` or an ssh command such as `ssh -p 2222 deploy@203.0.113.7` in the search box connects without saving a host. A **Connect** row for it heads the list and is selected, so Enter connects it; up and down move to the matching saved hosts. Plain words only search. If a saved host has the same address, port and user, that host is selected instead, so its key and jump host are used.

- `-p` and `-l` work; `-v`, `-C`, `-t`, `-4` and the like are ignored, and `-A`, `-X`, `-Y` connect without forwarding. Other options (`-i`, `-J`, `-L` …) and remote commands show a row saying why it cannot connect; save a host for keys, jump hosts or forwards.
- Without a user name, this computer's user name is used, as with `ssh`.
- It connects through this device's default proxy, if Settings → Proxies has one.
- Sign-in works like the `ssh` command: the SSH agent first, then the server's own prompts, or a password dialog (also when the server's prompts fail). The password is used once and never saved. Saved keys are not tried. A server that hangs up after rejecting too many agent keys is connected to again without the agent. The host key is checked as for any host (trust it on first use; it syncs).
- The tab is titled `user@host`. Port forwarding is not offered, and **More actions** has **Save as Host…**, which opens the host editor filled in with the address, port and user. Once the host is saved, **Reconnect** connects the tab as that host.
- Nothing is saved as a host by itself. Targets that connected are kept as **Recent** on this device only (at most 8, no passwords, never synced). With the search box empty and focused, they show under it (**Recent Quick Connections**); while typing, matching ones show as extra **Connect** rows. The × button removes one.

Other labels: **Not saved**, **Remove from Recent**, **Username**.

## Import hosts

**Import**, in the header of the host list and in the empty list, opens a menu: **SSH Config** reads `~/.ssh/config` (on Windows `%USERPROFILE%\.ssh\config`), and Xshell, FinalShell, MobaXterm, PuTTY, SecureCRT and Tabby read that client's saved sessions where it keeps them on this device. The dialog lists the SSH hosts and imports the ones you tick. Entries whose name already exists are unticked and marked **Already exists**. For sessions kept elsewhere or brought from another computer, **Choose File…** or **Choose Folder…** reads them from there:

- Xshell: the sessions folder in `Documents\NetSarang Computer\<version>\Xshell\Sessions`, a sessions folder kept elsewhere, or an `.xts` file from Xshell's **File → Export**.
- FinalShell: its data folder (`%LOCALAPPDATA%\finalshell` on Windows, `~/Library/FinalShell` on macOS, `~/.finalshell` on Linux), or a copy of it.
- MobaXterm: `%APPDATA%\MobaXterm\MobaXterm.ini`, the `MobaXterm.ini` beside the portable edition, or a `.mxtsessions` file from **Export all sessions to file**.
- PuTTY: the saved sessions in the registry on Windows and in `~/.putty/sessions` on macOS and Linux, or a `.reg` file of the key `HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions` exported in Registry Editor.
- SecureCRT: the `Sessions` folder in SecureCRT's configuration folder (`%APPDATA%\VanDyke\Config` on Windows, `~/Library/Application Support/VanDyke/SecureCRT/Config` on macOS, `~/.vandyke/SecureCRT/Config` on Linux), an `.xml` file from **Tools → Export Settings**, a configuration folder kept elsewhere or copied from another computer, or its `Sessions` folder.
- Tabby: `config.yaml` in Tabby's folder (`%APPDATA%\tabby`, `~/Library/Application Support/tabby`, `~/.config/tabby`). A config encrypted with Tabby's vault can't be read until its encryption is turned off in Tabby.

What comes along:

- Name, address, port and user, and the session's description as the note. A folder becomes a group named after its path, such as `Prod / DB`; a group with that name is reused.
- Jump hosts. A ProxyJump, or a Tabby, PuTTY or SecureCRT session that goes through another session (in SecureCRT, a session chosen as its firewall), is kept when that host exists in Hatoba or is imported with it (otherwise the import warns). A MobaXterm SSH gateway or a PuTTY SSH proxy given by address uses the saved host with that address, port and user, or adds one that signs in with the SSH agent, as a jump host can't ask for a password.
- SOCKS5 and HTTP proxies become saved proxies; one with the same address, port, user and password is reused. Other proxy settings (ProxyCommand, SOCKS4, Telnet, a local command, Xshell's proxies, which Xshell keeps itself, and the firewalls set in SecureCRT's Global Options) are not imported: the entry shows **ProxyCommand not imported** or **Proxy not imported**, the import warns, and the host connects without it until a proxy or jump host is set.
- SetEnv becomes the host's environment variables: the first SetEnv line that applies counts, as in OpenSSH, and a variable Hatoba can't use (see Environment variables below) is left out with a warning. SendEnv is not imported.
- Saved passwords are not imported.

Entries show their private key file (struck through if missing). Only the checkbox under the list (off by default), which lists the files, imports private keys into the encrypted vault, PuTTY `.ppk` files included; passphrase-protected ones are skipped. A SecureCRT session brings its own key file, or the one set for all sessions in SecureCRT's Global Options when it uses the global setting; an export from **Tools → Export Settings** holds those key files, so they import from it on another computer too. Otherwise hosts use **Ask Each Time**, or the SSH agent for a Tabby profile set to it. Keys that Xshell or FinalShell keep themselves, or that are in Tabby's vault, are not imported: the entry shows **Key stays in Xshell** (or the client's name), and the key can be exported from that client and imported on the Keys page.

Other labels: **Select All**, **Select None**.

## The host editor

**New Host**, **Edit** in the row menu, or **Edit Host** (in a terminal tab's **More actions** menu, or on the connection-failed card) opens the editor on the home tab. **Save** (or Ctrl+S, or Enter in a text field) saves, **Cancel** or the back arrow leaves, and Esc leaves an untouched form. The star in the header toggles the favorite. The address is required (it is the default name), and the port must be 1 to 65535.

Other labels: **Username**, **Organization & Network**.

- **Test Connection** connects with the current form values (saved or not), authenticates, checks the host key (you may be asked to trust it), shows the latency, and disconnects. It does not save the host.
- **Delete Host** is at the bottom of the editor for an existing host.
- **Notes** are for you only. **AI Notes** go to the AI assistant in every request of a conversation on the host; see `references/ai-instructions.md`.
- **Port Forwarding** in the editor lists the host's forwards; see `references/port-forwarding.md`. A new host must be saved before forwards can be added.
- **Environment Variables** are sent to the server when a terminal opens; see Environment variables below.

## Authentication methods

**Method** chooses how Hatoba signs in:

- **Password**: stored encrypted in the vault. After saving, the field shows **Saved** and the password can be replaced (**Replace**, **Keep Saved Password**) but never viewed.
- **Key**: choose a key from the Keys page (**Choose a key…**; **Manage Keys** opens the Keys page). RSA, Ed25519 and ECDSA keys work. If the key has a passphrase that was saved with it, it is used; otherwise Hatoba asks for the passphrase when connecting (**Enter the key passphrase**), and does not save it.
- **Ask Each Time**: Hatoba asks for the password at every connection and never saves it.
- **SSH Agent**: uses the keys in an SSH agent. On Windows: the OpenSSH Authentication Agent service (it must be running), or Bitwarden's or 1Password's agent in its place. On macOS and Linux: the agent `ssh` uses, named by `IdentityAgent` in `~/.ssh/config` or by `SSH_AUTH_SOCK`, also when only `~/.zshrc` or `~/.bashrc` exports it. Pageant is not supported.
- Servers that ask for a second factor (keyboard-interactive, such as a one-time code) show a dialog titled **Additional verification required** with a **Verify** button. This needs no setting.
- Deleting a key switches the hosts that used it to **Ask Each Time**.

## Jump host

**Jump Host** (ProxyJump) routes the connection through another saved host. Pick it from the list; the list excludes the host itself, and a loop of jump hosts is rejected. A jump host can have its own jump host. Each hop uses its own saved settings, and a jump host must use a saved password, a key with its passphrase saved, or **SSH Agent**. Behind a jump host, the jump host's own **Proxy** setting applies. The status bar of the tab shows "via" and the jump host's name.

## Proxy

**Proxy** sends the connection through a SOCKS5 or HTTP proxy. The proxy looks up the server's name, so a name that resolves only on the proxy's side works. The choices:

- **Device default**: the proxy chosen in Settings → Proxies on this device, or no proxy when none is chosen there. The picker names the proxy it stands for. New and imported hosts start with it.
- **No proxy**: always connect directly.
- A saved proxy, by name.

With a jump host, the jump host's own choice applies and the picker is disabled. Quick connections use this device's default. The reachability dots of the host list go through the same proxy as the connection. **Manage Proxies** opens Settings → Proxies, where proxies are added, edited and deleted (`references/settings.md`). Deleting a proxy switches the hosts that named it to **Device default**. When the proxy of a host, or this device's default, was deleted on another device, the picker shows **Deleted proxy** and connecting fails until another one is chosen.

## Environment variables

**Environment Variables** in the host editor lists `NAME=value` pairs, like `SetEnv` in OpenSSH. **Add Variable** adds a row and × removes one; they are saved with the host by **Save** and sync with it. When a terminal opens, Hatoba asks the server to set each one before the shell starts.

- The server sets only the names its sshd `AcceptEnv` setting lists and ignores the rest; the shell still starts, without them. Many servers (Debian and Ubuntu by default) accept only `LANG` and `LC_*`. To use other names, the server's administrator adds them to `AcceptEnv` in `/etc/ssh/sshd_config` and reloads sshd. Running `env` or `echo $NAME` in the terminal shows what arrived.
- Hatoba sends `LANG=C.UTF-8` by default; a host's `LANG` replaces it. `TERM` sets the terminal type instead (`xterm-256color` by default) and needs no `AcceptEnv`.
- Only the terminal's shell gets them. SFTP, the resource usage readout and the AI assistant's commands run without them.
- A name uses letters, digits and `_`, doesn't start with a digit, is case-sensitive, and appears once per host. A value can't contain a line break or another control character. A host can have at most 64 variables, 16 KB of names and values together.

## Connecting, host keys and prompts

A connection opens a new terminal tab (orange dot while connecting). The connection attempt times out after 15 seconds. Dialogs that can appear:

- First connection to a server: compare the shown fingerprint with the administrator's, then **Trust and Connect** (saved to the vault and synced) or **Cancel**.
- **New host key for …** appears when a hop offers none of the key types saved for it, and **Host key has changed** blocks a saved type's changed key (**Disconnect** is the default). See `references/troubleshooting-connections.md` for both.
- **Ask Each Time** hosts show a password prompt; **Connect** continues. A wrong key passphrase asks again.

Other labels: **Fingerprint**, **Trusted fingerprint**, **Fingerprint received now**, **Update Fingerprint and Connect**, **Key passphrase**.

## When a connection fails or ends

- A failed attempt shows a card over the terminal titled "Can’t connect to" plus the host name. It gives the reason, a short code (for example ETIMEDOUT) with the time, and the buttons **Copy Diagnostics**, **Ask AI** (opens the AI panel with the diagnostics attached; nothing is sent until the user sends it, see `references/ai-attachments.md`), **Edit Host** and **Retry**. Diagnostics hold the host, user, auth method, error and time, and no secrets. A quick connection has no **Edit Host** button. See `references/troubleshooting-connections.md` for each reason.
- A connection that ends later (the server closed it, the network dropped, keepalives went unanswered) shows the banner **Connection closed** with **Reconnect**. Hatoba never reconnects by itself. **More actions** has Reconnect and Disconnect too.
- Closing a tab (the × on the tab, middle-click, or Ctrl+Shift+W / ⌘W) ends its session.
- Locking keeps open sessions connected in the background unless **Disconnect all sessions when locked** is on (Settings → Security).
