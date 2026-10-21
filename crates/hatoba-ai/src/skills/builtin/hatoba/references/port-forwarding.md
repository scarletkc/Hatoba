# Port forwarding

Hatoba supports local port forwarding only, the equivalent of `ssh -L`. A port on this computer is forwarded through the SSH connection to an address that the server can reach. Remote forwarding (-R) and SOCKS / dynamic forwarding (-D) are not supported.

Forwards belong to a host. They are saved in the vault and sync like other items. On this device a forward runs in a terminal tab connected to the host, or without a terminal once started from the host editor.

## Create or edit a rule

Open the host in the editor (**Edit** in the host's menu) and find the **Port Forwarding** section. A new host must be saved first ("Save the host to add port forwards."). Rules are saved the moment you add, change or delete them; the host form's **Save** button is not involved.

Other labels: **Add Forward…**, **Add Port Forward**, **Edit Port Forward**, **Add**, **Delete**.

- Each rule reads `local address:port → destination:port`. Use the pencil to edit and the bin to delete (with confirmation; with Cloud Sync on it is removed on other devices too).
- **Local address** is the address Hatoba listens on, and its port. `127.0.0.1` is reachable from this computer only. `0.0.0.0` exposes the port to the whole network, and the dialog warns about it. A local port of 0 picks a free port automatically (shown as **auto**).
- **Destination** is the host and port that the server connects to. `127.0.0.1` means the server itself. It can also be another machine the server can reach, such as an internal address.
- **Start when this host connects** (the **Auto-start** switch in the list) starts the rule as soon as the connection is up. Rules without it stay stopped until started by hand.

## Start and stop in a session

In a connected terminal tab, **Port Forwarding** in the status bar opens a popover with the host's rules. A badge on the button shows how many are running. The button is disabled until the tab is connected.

The popover can also say:

- "This host has no port forwards yet."
- "Can’t read the port forwards."

- Each row shows the state (stopped, running, failed), the local address and the destination. A running rule shows `localhost:<port>`; click it to copy the address (use it in a browser or client on this computer).
- **Start** starts a rule, **Stop** stops it, and **Retry** starts a failed one again. A failed rule shows the reason, for example that the local port is already in use.
- **Manage Port Forwarding…** goes to the host editor.
- Closing the tab or disconnecting stops its forwards.

## Run without a terminal

**Start** on a rule in the host editor's **Port Forwarding** section runs it with no terminal tab open. Hatoba opens a connection of its own to the host, without a shell, which the host's other rules started this way share. It signs in as a terminal tab does and asks the same questions: an "ask every time" password, a key passphrase that is not saved, a new host key, or the server's two-factor prompts.

- A running rule shows `localhost:<port>` under it; click it to copy the address. **Stop** stops it, and the connection closes with the host's last running rule. A rule that could not start shows why, with **Retry**.
- A rule already running in a terminal tab shows "In a terminal tab", and **Stop** stops it in that tab.
- These forwards keep running with the window minimized to the tray (Settings → General → **Close Button**) and behind the lock screen, until they are stopped or Hatoba quits. With **Disconnect all sessions when locked** (Settings → Security) they stop when the vault locks.
- On Windows and Linux the tray menu lists them under **Port Forwarding**, each as the host's name with `localhost:<port> → destination`, and its submenu has **Copy Address** and **Stop**.
- In a terminal tab on the same host, the **Port Forwarding** popover shows such a rule as running "In the background", and **Stop** there stops it. A rule with **Start when this host connects** that already runs this way is not started again by the tab.
- When the connection is lost (the server or the network goes away), its forwards stop and Hatoba says so: a message in the window while Hatoba is the active window, otherwise a system notification titled with the host's name, "Port forwarding stopped." and the reason. Clicking the notification opens the host editor to start them again. Hatoba does not reconnect on its own.
- Deleting a rule stops it wherever it runs.

## Typical use

To reach a database on the server's private network from this computer: add a rule with local address `127.0.0.1`, local port `5433`, destination `db.internal` port `5432`, and **Start when this host connects**. After connecting, point the client at `localhost:5433`.
