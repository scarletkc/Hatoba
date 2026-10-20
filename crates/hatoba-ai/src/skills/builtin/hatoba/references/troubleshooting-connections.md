# Troubleshooting: SSH connections

Covers every kind of SSH failure, jump hosts, host key prompts, and dropped connections. Key and vault errors are in `references/troubleshooting.md`, sync errors in `references/troubleshooting-sync.md`, AI errors in `references/troubleshooting-ai.md`.

## Reading a failure

A failed connection shows a card over the terminal, titled "Can’t connect to" plus the host name, with the reason, a code line (`CODE · time`, or `CODE · N retries · time`), and the buttons **Copy Diagnostics**, **Ask AI**, **Edit Host** and **Retry**. The host list shows **Failed** on that host. **Test Connection** in the host editor gives the same reasons.

**Ask AI** opens the AI panel on that tab with these diagnostics attached as a chip ("Connection diagnostics · host"), and a suggested question in the input. Nothing is sent until the user sends the message. In the conversation they arrive as a `<connection_diagnostics host="…">` block before the question. **Copy Diagnostics** copies the same text to the clipboard. The text is English and holds no secrets (no password, key or passphrase), only the host's name, address, user and settings:

```
Hatoba connection diagnostics
Host: <name> (<address>:<port>)
User: <user>
Auth: password | key | ask | agent
Jump host: <name>
Proxy: socks5 | http <address>:<port> [(device default)]
Error: ETIMEDOUT (ssh/timeout)
Detail: <technical sentence>
Attempts: <n>
Time: <ISO time>
```

`Proxy:` names the proxy in use (`deleted` when it no longer exists). `Error:` has the code and, in brackets, the error class and SSH kind. `Detail:` is the useful part; for a chain of jump hosts it starts with `hop i/N (host:port):`. Read the kind first, then the detail. A failed tab has no connected terminal, so the assistant cannot run commands on the host or read its screen; it can explain the error, say what to check, and name the Hatoba setting to change (**Address**, **Port**, **Method**, **Jump Host**, **Proxy**, the key). It should not ask for passwords or keys.

## Each kind of failure

Messages that contain the host, port or seconds are paraphrased. "Edit" means **Edit Host** on the card, or the host editor.

| Kind (code) | Meaning and typical detail | What to check, and where |
|---|---|---|
| `dns` (ENOTFOUND) | "Can’t find the host …". Detail: `cannot resolve <host>: …` | A typo in **Address**; a name that only resolves on a VPN or company DNS; DNS down. Edit the address, try the IP, connect the VPN. For a host behind a jump host the jump host resolves the name, so DNS problems there show as `channel`; through a proxy the proxy resolves it |
| `refused` (ECONNREFUSED) | "<host>:<port> refused the connection". Detail: `connect to <ip:port>: …refused` | sshd is not running, or **Port** is wrong (default 22; a server may listen on another port), or a firewall rejects it. Edit the port; start sshd |
| `timeout` (ETIMEDOUT) | "<host>:<port> didn’t respond within 15 seconds". Detail: `timed out after 15 s while <phase>` where the phase is resolving the host name, establishing the TCP connection, the SSH handshake, authentication, or opening the tunnel channel | TCP phase: host down, wrong IP, or a firewall or cloud security group drops the port; try another network. Handshake: something other than sshd answers, or the server is overloaded. Authentication: very slow login (PAM, reverse DNS on the server). The limit applies to each hop; time spent in host key or verification-code dialogs does not count |
| `unreachable` (ENETUNREACH) | "The network can’t reach <host>". Detail ends with `Network is unreachable` or `No route to host` | This PC has no route: network off, VPN down, an IPv6-only address without IPv6, or a private address from outside its network |
| `auth_failed` (EAUTH) | See the messages below. Detail: `the server rejected the password`, `the server rejected the private key`, `ssh-agent holds no identities`, `none of the N ssh-agent identities was accepted by the server`, or `no usable authentication method`, often followed by `(server accepts: publickey, password, …)` | The `server accepts` list names the methods the server allows: if the chosen **Method** is not in it, switch (many servers disable passwords). Check **Username** and the password (**Replace** it). Key: the public key must be in `~/.ssh/authorized_keys` (**Deploy to Host…** on the Keys page). Agent: start the agent and add keys |
| `host_key_rejected` (EHOSTKEY) | See the messages below. Detail: `server host key was not accepted` | The host key dialog was declined (**Cancel** or **Disconnect**), or the key changed. See the host key section |
| `key_parse` (EKEY) | See the messages below. Method **Key** only | The key's format or passphrase. See `references/troubleshooting.md` |
| `disconnected` (ECONNRESET) | See the messages below. Detail: `connection closed by the remote side`, `server closed the connection: <message>`, `… (during authentication)`, `keepalive timeout: the server stopped responding`, `inactivity timeout`, `the connection through jump host … was lost` | The server or network ended it. During authentication: fail2ban or rate limits, `MaxAuthTries`, `MaxStartups`. Later: idle timeouts, a reboot, Wi-Fi or VPN change. Use **Reconnect** |
| `protocol` (EPROTO) | See the messages below. Detail mentions key exchange, a version, or no common algorithms | The port is not an SSH server (HTTP or a proxy answers), or the server is too old or too strict for Hatoba's algorithms. Check **Port** and the server's `sshd_config` |
| `io` (EIO) | See the messages below | A local or network read/write failure: network change, a local firewall or antivirus. Retry |
| `channel` (ECHANNEL) | See the messages below. Detail: `server refused to open the channel: …`, or `jump host could not open a tunnel to <host>:<port>` | Session limit (`MaxSessions`) or a user whose session is refused. With a jump host: the jump host forbids forwarding (`AllowTcpForwarding no`), or cannot reach the target |
| `sftp` (ESFTP) | See the messages below | The SFTP subsystem is disabled on the server, or a file operation failed; see `references/sftp.md` |
| `cancelled` (ECANCELED) | See the messages below | The connection was cancelled (a tab closed or a prompt cancelled). Nothing to fix |
| `proxy_unreachable` (EPROXY) | See the messages below. Detail: `cannot reach the proxy <address:port>: …`, or `timed out … while connecting to the proxy` | The proxy is not running, or its **Address** or **Port** (Settings → Proxies) is wrong. Start or fix it, or choose **No proxy** |
| `proxy_auth` (EPROXYAUTH) | See the messages below. Detail: `the proxy … requires a username and password`, `… rejected the username or password`, or `(HTTP 407)` | Set or fix the proxy's **Username** and **Password** in Settings → Proxies |
| `proxy` (EPROXY) | "The proxy couldn’t connect to <host>:<port>…". Detail: `did not answer as a SOCKS5 proxy`, `closed the connection during the … handshake`, `not allowed by its rules`, `(HTTP 403)`, `(HTTP 502)`, or `closed before the SSH handshake finished` | The wrong **Type** (SOCKS5 and HTTP often use different ports), the proxy's rules block the server, or the proxy cannot reach it |
| `proxy_missing` (EPROXY) | See the messages below. Detail: `the host's proxy was deleted` or `this device's default proxy was deleted` | Choose another **Proxy** in the host editor, or another default in Settings → Proxies |
| `other` (EFAILED) | See the messages below. Detail examples: `ssh-agent is not available (is the "OpenSSH Authentication Agent" service running?)`, `ssh-agent is not available at <socket>: …` | **SSH Agent** on Windows needs that service, or Bitwarden's or 1Password's agent, running. Elsewhere the agent must be running at the socket that `IdentityAgent` in `~/.ssh/config` or `SSH_AUTH_SOCK` names, which the detail shows |

The messages for these kinds:

- "Authentication failed: the server didn’t accept the username, password or key."
- "The host key was rejected, so the connection was stopped."
- "The private key can’t be read. Check its passphrase or format."
- "The connection was closed."
- "SSH protocol error."
- "Network read/write error."
- "Couldn’t open a session channel."
- "The SFTP subsystem isn’t available."
- "The connection was cancelled."
- "The connection failed."
- "Can’t reach the proxy. Check that it’s running and that its address and port are right."
- "The proxy asks for a username and password, or didn’t accept the saved ones."
- "The proxy this connection uses was deleted. Choose another one in the host’s settings or in Settings → Proxies."

Non-SSH reasons on the card: **Please check your input.** with code INVALID_INPUT means the host settings are unusable (the Detail says which, for example a deleted key or a missing password); "The vault is locked." means unlock first.

## Jump hosts (ProxyJump)

Set it in the host editor: **Organization & Network** → **Jump Host**, then pick a saved host. A jump host may have its own jump host, so chains work (up to 8 hops). The list leaves out the host itself, and saving is refused with these messages:

- "A host can’t be its own jump host."
- "The jump host chain can’t loop back on itself."

- Each hop connects and authenticates with its own saved settings (address, port, user, method). A jump host must use a saved password, a key with its passphrase saved, or **SSH Agent**. **Ask Each Time** and an unsaved key passphrase cannot be answered for a hop, so the connection fails with code INVALID_INPUT and the Detail "jump host <name> needs a saved password or key". Fix it in the jump host's own editor.
- Hatoba opens the next hop through a tunnel of the previous one. The destination's name is resolved and reached from the jump host, not from this PC. If the jump host cannot reach it, you get a `channel` failure: `jump host could not open a tunnel to <host>:<port>`.
- A chain's Detail starts with `hop i/N (host:port)`, counting from the first jump host; hop N is the final host. Hop 1 refused, timed out or failed authentication means the jump host itself. The 15-second limit is per hop.
- Each hop has its own host key prompts (below) and may ask for a verification code.
- If a jump host's connection dies later, the disconnect reason reads "the connection through jump host … was lost". The status bar shows "via" and the jump host's name.
- To isolate a problem, connect to the jump host alone, then use **Test Connection** on the final host.
- Not supported: `ProxyCommand`. Use a jump host, a proxy (below), or a system VPN or tunnel. Importing an SSH config keeps `ProxyJump` only when its first hop names a host that exists in Hatoba; otherwise the import warns.

## Proxies

The choices are in `references/hosts-and-connecting.md`. Only the first hop (the host, or its outermost jump host) goes through the proxy, and the proxy resolves the server's name. `refused`, `unreachable` or `timeout` with the Detail `the proxy could not connect to …` happened on the proxy's side. **Device default** follows each device's own choice. Through a proxy, the status bar's latency is an SSH keepalive round trip. To isolate a problem, set **Proxy** to **No proxy** and use **Test Connection** when the server is reachable directly.

## Host key prompts and changes

- First connection: compare the fingerprint with the server's, for example `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on the server. **Trust and Connect** saves and syncs it; **Cancel** fails with `host_key_rejected`.
- Hatoba asks for a key type already saved for the host:port, so adding a type on the server changes nothing. **New host key for …** means it offers none: the old type was removed, or another machine answers there. The other type's trusted key is shown for reference; it never matches. Verify the new one as on a first connection; trusting it keeps the other.
- **Host key has changed**: a saved type's key differs, after a reinstall or new keys, another machine at that address (reused IP, load balancer), or a man-in-the-middle. **Disconnect** is the default. When the change is explained, **Update Fingerprint and Connect** replaces that type's fingerprint on all synced devices. Saved fingerprints have no list or delete screen.

## Dropped connections and keepalive

When a session ends after it was connected, the tab turns grey with the banner **Connection closed** and a reason in brackets, then **Reconnect**. Hatoba never reconnects by itself, and a reconnect starts a new shell (use tmux or screen on the server to keep work).

Other labels: **Disconnected**.

- Hatoba sends an SSH keepalive every 30 seconds. When 3 go unanswered the session is closed with `keepalive timeout: the server stopped responding`, which is typically about 90 seconds after the network died. A backstop closes a connection that received nothing for 150 seconds. After sleep, Wi-Fi or VPN changes, the banner appears within a minute or two.
- `server closed the connection: …` or `connection closed by the remote side` means the server ended it (idle timeout such as `ClientAliveInterval`, an admin, a restart, or fail2ban).
- Locking the vault keeps sessions connected unless **Disconnect all sessions when locked** is on; see `references/keys-vault-and-lock.md`.
