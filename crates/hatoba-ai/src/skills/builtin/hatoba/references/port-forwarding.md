# Port forwarding

Hatoba forwards ports the three ways `ssh` does:

- **Local** (`ssh -L`): a port on this computer leads through the SSH connection to an address that the server can reach.
- **Remote** (`ssh -R`): a port on the server leads back through the connection to an address that this computer can reach.
- **Dynamic (SOCKS)** (`ssh -D`): a SOCKS proxy on this computer. The server opens each connection that an app makes through it, and resolves the names. It speaks SOCKS5 without a username or password, SOCKS4 and SOCKS4a.

Forwards belong to a host. They are saved in the vault and sync like other items. On this device a forward runs on the host's connection, which the host's terminal tabs share, so a rule is running or stopped in the same way on the **Port Forwarding** page, in a tab and in the tray menu.

## Create or edit a rule

**Port Forwarding** in the sidebar opens a page with every host's rules, a section per host with its name and `user@address:port`; with none yet it says "No port forwards yet". **Add Forward…** opens **Add Port Forward**, whose **Host** is the saved host to forward through ("Choose a host." until one is chosen), and whose **Type** is **Local**, **Remote** or **Dynamic (SOCKS)**; the fields below follow the type. Rules are saved the moment you add, change or delete them.

Other labels: **Edit Port Forward**, **Add**, **Delete**.

- **Name** is optional: a word or two on what the rule is for, such as "Postgres". A named rule shows its name with the rule under it, on the page, in the popover and in the tray menu, and the delete confirmation names it; a rule without one shows only the rule. Renaming a running rule renames it at once, and it keeps running.
- A local rule reads `local address:port → destination:port`, with a loopback destination such as `127.0.0.1` shown "on the server", a remote one `address:port on the server → destination:port`, and a SOCKS one `local address:port` followed by "SOCKS proxy". An edited rule stays on its host, and can change its type. Use the pencil to edit and the bin to delete (with confirmation; with Cloud Sync on it is removed on other devices too). Deleting a running rule stops it, also when it is deleted on another device.
- **Local address** (local and SOCKS rules) is the address Hatoba listens on, and its port. `127.0.0.1` is reachable from this computer only. `0.0.0.0` exposes the port to the whole network, and the dialog warns about it. A port of 0 picks a free port automatically (shown as **auto**).
- **Server address** (remote rules) is the address the server listens on, and its port. It starts as `localhost`, the server's own loopback, as with `ssh -R`. Whether another address such as `0.0.0.0` lets other machines reach the port is up to the server's `GatewayPorts` setting; OpenSSH's default keeps the port on loopback, and the dialog says so. A port of 0 lets the server pick one. Ports below 1024 usually need root on the server.
- **Destination** is the host and port that the server connects to; `127.0.0.1` means the server itself, and it can be another machine the server can reach, such as an internal address. For a remote rule it is the host and port that this computer connects to, and `127.0.0.1` is this computer. A SOCKS rule has no destination: each app names its own.
- **Start when this host connects** (the **Start on connect** switch in the list) starts the rule whenever Hatoba connects to the host: when a terminal tab, or a rule started on the page, opens the host's connection. Rules without it stay stopped until started by hand.

## Start and stop

**Start** on a rule on the **Port Forwarding** page starts it (the rule shows "Starting…" until it runs or fails), **Stop** stops it, and **Retry** starts a failed one again.

- With nothing open on the host, starting a rule connects to it without opening a terminal. Hatoba signs in as a terminal tab does and asks the same questions: an "ask every time" password, a key passphrase that is not saved, a new host key, or the server's two-factor prompts. With a tab or another forward already on the host, it asks nothing.
- A running local or SOCKS rule shows its address, `localhost:<port>` (or the address it listens on when that is one network interface's); click it to copy it, for a browser or client on this computer, or as an app's SOCKS proxy. A running remote rule shows its address "on the server", with nothing to copy: programs on the server connect to it.
- A rule that could not start shows why, for example that the local port is already in use, or that the server did not listen on a remote rule's port (in use there, a port that needs root, or a server that does not allow remote forwarding, `AllowTcpForwarding`). Stopping a remote rule asks the server to stop listening.
- In a terminal tab on a host that has rules, **Port Forwarding** in the status bar opens a popover with them; a badge on the button shows how many are running. The rows have the same **Start**, **Stop** and **Retry**, and **Manage Port Forwarding…** opens the page at this host's rules. The page can also say "Can’t read the port forwards."
- Closing a terminal tab or disconnecting it closes only its shell. The forwards keep running, and the host's connection closes once no tab and no forward is left on it.

## Keep running in the background

- Forwards keep running with the window minimized to the tray (Settings → General → **Close Button**) and behind the lock screen, until they are stopped, the connection ends, or Hatoba quits. With **Disconnect all sessions when locked** (Settings → Security) they stop when the vault locks.
- On Windows and Linux the tray menu lists them under **Port Forwarding**, each as the host's name, the rule's name if it has one, and the rule as running (`localhost:<port> → destination`, `address on the server → destination`, or `localhost:<port>` and "SOCKS proxy"). Its submenu has **Stop**, and **Copy Address** for a local or SOCKS rule.
- When the connection is lost (the server or the network goes away), its forwards stop and Hatoba says so: a message in the window while Hatoba is the active window, otherwise a system notification titled with the host's name, "Port forwarding stopped." and the reason. Clicking the notification opens the **Port Forwarding** page at the host's rules to start them again. The host's terminal tabs show the disconnect too. Hatoba does not reconnect on its own.

## Typical use

- To reach a database on the server's private network from this computer: a **Local** rule with local address `127.0.0.1`, port `5433`, destination `db.internal` port `5432`, and **Start when this host connects**. After connecting, point the client at `localhost:5433`.
- To let the server reach an app running on this computer, such as a web server on port `3000` that should receive webhooks: a **Remote** rule with server address `localhost`, port `8080`, destination `127.0.0.1` port `3000`. On the server, `localhost:8080` then reaches the app.
- To browse through the server's network: a **Dynamic (SOCKS)** rule on `127.0.0.1` port `1080`, then set the browser's or the system's SOCKS proxy to `localhost:1080` (SOCKS5; where the app offers it, let the proxy resolve names, so the server looks up internal names).
