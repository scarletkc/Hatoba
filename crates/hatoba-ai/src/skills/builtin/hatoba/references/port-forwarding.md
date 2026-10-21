# Port forwarding

Hatoba forwards ports the three ways `ssh` does:

- **Local** (`ssh -L`): a port on this computer leads through the SSH connection to an address that the server can reach.
- **Remote** (`ssh -R`): a port on the server leads back through the connection to an address that this computer can reach.
- **Dynamic (SOCKS)** (`ssh -D`): a SOCKS proxy on this computer. The server opens each connection that an app makes through it, and resolves the names. It speaks SOCKS5 without a username or password, SOCKS4 and SOCKS4a.

Forwards belong to a host. They are saved in the vault and sync like other items. On this device a forward runs in a terminal tab connected to the host, or without a terminal once started on the **Port Forwarding** page.

## Create or edit a rule

**Port Forwarding** in the sidebar opens a page with every host's rules, a section per host with its name and `user@address:port`; with none yet it says "No port forwards yet". **Add Forward…** opens **Add Port Forward**, whose **Host** is the saved host to forward through ("Choose a host." until one is chosen), and whose **Type** is **Local**, **Remote** or **Dynamic (SOCKS)**; the fields below follow the type. Rules are saved the moment you add, change or delete them.

Other labels: **Edit Port Forward**, **Add**, **Delete**.

- A local rule reads `local address:port → destination:port`, a remote one `address:port on the server → destination:port`, and a SOCKS one `local address:port` followed by "SOCKS proxy". An edited rule stays on its host, and can change its type. Use the pencil to edit and the bin to delete (with confirmation; with Cloud Sync on it is removed on other devices too).
- **Local address** (local and SOCKS rules) is the address Hatoba listens on, and its port. `127.0.0.1` is reachable from this computer only. `0.0.0.0` exposes the port to the whole network, and the dialog warns about it. A port of 0 picks a free port automatically (shown as **auto**).
- **Server address** (remote rules) is the address the server listens on, and its port. It starts as `localhost`, the server's own loopback, as with `ssh -R`. Whether another address such as `0.0.0.0` lets other machines reach the port is up to the server's `GatewayPorts` setting; OpenSSH's default keeps the port on loopback, and the dialog says so. A port of 0 lets the server pick one. Ports below 1024 usually need root on the server.
- **Destination** is the host and port that the server connects to; `127.0.0.1` means the server itself, and it can be another machine the server can reach, such as an internal address. For a remote rule it is the host and port that this computer connects to, and `127.0.0.1` is this computer. A SOCKS rule has no destination: each app names its own.
- **Start when this host connects** (the **Auto-start** switch in the list) starts the rule as soon as the connection is up. Rules without it stay stopped until started by hand.

## Start and stop in a session

In a connected terminal tab, **Port Forwarding** in the status bar opens a popover with the host's rules. A badge on the button shows how many are running. The button is disabled until the tab is connected.

The popover can also say:

- "This host has no port forwards yet."
- "Can’t read the port forwards."

- Each row shows the state (stopped, running, failed), where the rule listens and where it leads, or "SOCKS proxy". A running local or SOCKS rule shows `localhost:<port>`; click it to copy the address (use it in a browser or client on this computer, or as the app's SOCKS proxy). A running remote rule shows its address "on the server", with nothing to copy: programs on the server connect to it.
- **Start** starts a rule, **Stop** stops it, and **Retry** starts a failed one again. A failed rule shows the reason, for example that the local port is already in use, or that the server did not listen on a remote rule's port (in use there, a port that needs root, or a server that does not allow remote forwarding, `AllowTcpForwarding`).
- **Manage Port Forwarding…** opens the **Port Forwarding** page at this host's rules.
- Closing the tab or disconnecting stops its forwards. Stopping a remote rule asks the server to stop listening.

## Run without a terminal

**Start** on a rule on the **Port Forwarding** page runs it with no terminal tab open. Hatoba opens a connection of its own to the host, without a shell, which the host's other rules started this way share. It signs in as a terminal tab does and asks the same questions: an "ask every time" password, a key passphrase that is not saved, a new host key, or the server's two-factor prompts.

- A running rule shows where it listens under it: `localhost:<port>` to copy for a local or SOCKS rule, the address on the server for a remote one. **Stop** stops it, and the connection closes with the host's last running rule. A rule that could not start shows why, with **Retry**.
- A rule already running in a terminal tab shows "In a terminal tab", and **Stop** stops it in that tab.
- These forwards keep running with the window minimized to the tray (Settings → General → **Close Button**) and behind the lock screen, until they are stopped or Hatoba quits. With **Disconnect all sessions when locked** (Settings → Security) they stop when the vault locks.
- On Windows and Linux the tray menu lists them under **Port Forwarding**, each as the host's name with its rule as running (`localhost:<port> → destination`, `address on the server → destination`, or `localhost:<port>` and "SOCKS proxy"). Its submenu has **Stop**, and **Copy Address** for a local or SOCKS rule.
- In a terminal tab on the same host, the **Port Forwarding** popover shows such a rule as running "In the background", and **Stop** there stops it. A rule with **Start when this host connects** that already runs this way is not started again by the tab.
- When the connection is lost (the server or the network goes away), its forwards stop and Hatoba says so: a message in the window while Hatoba is the active window, otherwise a system notification titled with the host's name, "Port forwarding stopped." and the reason. Clicking the notification opens the **Port Forwarding** page at the host's rules to start them again. Hatoba does not reconnect on its own.
- Deleting a rule stops it wherever it runs.

## Typical use

- To reach a database on the server's private network from this computer: a **Local** rule with local address `127.0.0.1`, port `5433`, destination `db.internal` port `5432`, and **Start when this host connects**. After connecting, point the client at `localhost:5433`.
- To let the server reach an app running on this computer, such as a web server on port `3000` that should receive webhooks: a **Remote** rule with server address `localhost`, port `8080`, destination `127.0.0.1` port `3000`. On the server, `localhost:8080` then reaches the app.
- To browse through the server's network: a **Dynamic (SOCKS)** rule on `127.0.0.1` port `1080`, then set the browser's or the system's SOCKS proxy to `localhost:1080` (SOCKS5; where the app offers it, let the proxy resolve names, so the server looks up internal names).
