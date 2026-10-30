# Resource usage

**Resource Usage** on the hosts page (the switch in the row under its header) shows the online status and resource usage of the listed hosts, without opening a tab on each. It reads Linux and macOS servers over SSH, as the terminal's resource usage does (see `references/terminal.md`): nothing is installed on the server and no Hatoba server is involved. The readings stay in memory on this device; they are never stored or synced.

Other labels: **Filesystems**.

## On the hosts page

- It reads the hosts the page lists: every host, or a group's, narrowed by **Favorites** and **Tags**. The search box does not change which hosts are read. The switch is kept on this device.
- In **Cards**, a card shows the name, `user@address:port`, tags, the OS icon with a status badge, and CPU, memory and disk as bars, then the network rates, the load averages and the uptime. A host without readings shows its status in their place, with the reason and **Retry**.
- In **List**, a row has the status, CPU, memory, disk, network, load and uptime. **Columns** turns each of them on or off; a narrow window drops the last ones.
- Disk is the root filesystem, or the fullest one once it is over the disk threshold. Pointing at it lists the filesystems.
- The sort order adds status (problems first), CPU, memory, disk (the fullest filesystem), network and uptime. Values sort highest first. A search keeps its own order.
- A value over its threshold shows in red. The thresholds are in Settings → General (90% by default; **Off** turns one off).
- A host that stops answering keeps its last readings, dimmed, with the time they were read.
- **Show Details** in a row's or card's right-click menu, or Space on the selected one, shows the host's detail: CPU and network over the last 3 minutes, memory and swap, the filesystems, the total traffic of the counted interfaces (since each came up, usually at boot), the load and uptime. **Open Terminal** brings the host's tab to the front or opens one. Double-click, Enter and **Connect** connect as on the hosts page without resource usage.
- The filesystems are those that store files, at most 32: not memory-backed or kernel ones, container layers, read-only images such as snap packages, or the files a container runtime mounts, and each disk, each ZFS pool and each APFS container once. On macOS they are `/` and the filesystems Finder shows, without read-only ones. They are read every fifth reading.

## How hosts are read

- Hosts are read only while Resource Usage is on, the hosts page is shown and the vault is unlocked, also while the window is minimized. Leaving the page or turning the switch off stops reading, and the connections close a minute later, so a look at a tab and back does not sign in again. Locking closes them at once.
- Each host is read every 3 seconds by default (**Refresh interval** in Settings → General), over the host's open connection (its terminal tabs' and port forwards') or else over a background SSH connection of its own. At most 8 connections are being set up at once, which keeps many hosts behind one jump host under OpenSSH's default `MaxStartups`. Until a jump host has let one connection through, the hosts behind it try it one at a time.
- A host whose first hop (the host, or its outermost jump host) does not answer the online probe is shown **Offline** and is not signed in to until it answers; it is checked again every minute, or at once with **Retry** in its menu.
- When a host's connection settings change (on another device, say), Hatoba connects again with the new ones and drops the readings made with the old ones.
- **Opening a tab** or starting a forward on a host that is being read takes over that connection: a tab opens its shell without signing in again, then sets the environment variables, and the forwards set to start with the connection start. Once no tab and no forward is left on it, the connection closes and the reading connects again.
- Servers that run neither Linux nor macOS show their online status only. Hatoba signs in once to find that out, and again only after the vault was locked or the host's connection settings changed.

Other labels: **Online**, **Waiting**, **Read over an open connection**, **Highlight CPU usage above**, **Highlight memory usage above**, **Highlight disk usage above**.

## Hosts that need the user

The background never shows a prompt. A host that cannot sign in on its own says why, with **Connect**, which opens a tab where the prompt can be answered. Once the tab is connected, the host is read over it at once.

Other labels: **Asks for a password on every connection**, **The key’s passphrase isn’t saved**, **Asks for a password or verification code**, **Host key not confirmed yet**, **Host key changed**, **Sign-in failed**.

- A failed sign-in, with the server's answer, is not tried again on its own until the vault is locked and unlocked, so tools like fail2ban do not ban this computer. That includes a server that hangs up during sign-in, as one does after too many keys from the agent. **Retry** tries once more. Changing how the host connects (a new password, key, user, address or jump host, say) or confirming its host key in a tab also lets the reading try again; renaming it or marking it a favorite does not.
- When a jump host turns one host away (or turns the reading away when the jump host itself is listed), every host behind it waits too, so its rejected password is not tried once per host. **Retry** on any of them tries again from that host only; the others follow once it gets past the jump host.
- Other failures, such as a timeout or a lost connection, show the error and are tried again after a minute; **Retry** tries at once.

## Leaving hosts out

With Resource Usage on, right-click a row or card for **Don’t Read Usage**, or right-click a group's button (or use the **…** beside a group's name) for **Don’t Read the Group’s Usage**. A host left out stays listed, dimmed and marked **Not Read**, and is not signed in to; **Read Usage**, or **Read the Group’s Usage** for a host whose group is left out, brings it back. The choice is kept on this device only and is not synced.

## Not in resource usage yet

History beyond the last few minutes, notifications, billing and traffic per billing cycle, temperatures, GPUs, Docker containers, custom commands per host, and a card for this computer. Readings stop when Hatoba is closed, the page is left or the switch is turned off: nothing is left running on the server.
