# Port forwarding

Hatoba supports local port forwarding only, the equivalent of `ssh -L`. A port on this computer is forwarded through the SSH connection to an address that the server can reach. Remote forwarding (-R) and SOCKS / dynamic forwarding (-D) are not supported.

Forwards belong to a host. They are saved in the vault and sync like other items. On this device a forward runs on the host's connection, which the host's terminal tabs share, so a rule is running or stopped in the same way on the **Port Forwarding** page, in a tab and in the tray menu.

## Create or edit a rule

**Port Forwarding** in the sidebar opens a page with every host's rules, a section per host with its name and `user@address:port`; with none yet it says "No port forwards yet". **Add Forward…** opens **Add Port Forward**, whose **Host** is the saved host to forward through ("Choose a host." until one is chosen). Rules are saved the moment you add, change or delete them.

Other labels: **Edit Port Forward**, **Add**, **Delete**.

- Each rule reads `local address:port → destination:port`. An edited rule stays on its host. Use the pencil to edit and the bin to delete (with confirmation; with Cloud Sync on it is removed on other devices too). Deleting a running rule stops it.
- **Local address** is the address Hatoba listens on, and its port. `127.0.0.1` is reachable from this computer only. `0.0.0.0` exposes the port to the whole network, and the dialog warns about it. A local port of 0 picks a free port automatically (shown as **auto**).
- **Destination** is the host and port that the server connects to. `127.0.0.1` means the server itself. It can also be another machine the server can reach, such as an internal address.
- **Start when this host connects** (the **Auto-start** switch in the list) starts the rule whenever Hatoba connects to the host: when a terminal tab, or a rule started on the page, opens the host's connection. Rules without it stay stopped until started by hand.

## Start and stop

**Start** on a rule on the **Port Forwarding** page starts it, **Stop** stops it, and **Retry** starts a failed one again.

- With nothing open on the host, starting a rule connects to it without opening a terminal. Hatoba signs in as a terminal tab does and asks the same questions: an "ask every time" password, a key passphrase that is not saved, a new host key, or the server's two-factor prompts. With a tab or another forward already on the host, it asks nothing.
- A running rule shows its address, `localhost:<port>` (or the address it listens on when that is one network interface's); click it to copy it, for a browser or client on this computer. A rule that could not start shows why, for example that the local port is already in use.
- In a terminal tab on a host that has rules, **Port Forwarding** in the status bar opens a popover with them; a badge on the button shows how many are running. The rows have the same **Start**, **Stop** and **Retry**, and **Manage Port Forwarding…** opens the page at this host's rules. The popover can also say "Can’t read the port forwards."
- Closing a terminal tab or disconnecting it closes only its shell. The forwards keep running, and the host's connection closes once no tab and no forward is left on it.

## Keep running in the background

- Forwards keep running with the window minimized to the tray (Settings → General → **Close Button**) and behind the lock screen, until they are stopped, the connection ends, or Hatoba quits. With **Disconnect all sessions when locked** (Settings → Security) they stop when the vault locks.
- On Windows and Linux the tray menu lists them under **Port Forwarding**, each as the host's name with `localhost:<port> → destination`, and its submenu has **Copy Address** and **Stop**.
- When the connection is lost (the server or the network goes away), its forwards stop and Hatoba says so: a message in the window while Hatoba is the active window, otherwise a system notification titled with the host's name, "Port forwarding stopped." and the reason. Clicking the notification opens the **Port Forwarding** page at the host's rules to start them again. The host's terminal tabs show the disconnect too. Hatoba does not reconnect on its own.

## Typical use

To reach a database on the server's private network from this computer: add a rule with local address `127.0.0.1`, local port `5433`, destination `db.internal` port `5432`, and **Start when this host connects**. After connecting, point the client at `localhost:5433`.
