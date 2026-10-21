# Dashboard

**Dashboard** at the top of the sidebar shows the online status and resource usage of every saved host on one page, without opening a tab on each. It reads Linux servers over SSH, as the terminal's resource usage does (see `references/terminal.md`): nothing is installed on the server and no Hatoba server is involved. The readings stay in memory on this device; they are never stored or synced.

Other labels: **No Group**, **Filesystems**.

## The page

- The header has a filter (All Hosts, Favorites, a group or a tag, as in the sidebar), the sort order, and **Cards** or **Table**. The page shows a section per group, in the sidebar's order, then the hosts without a group.
- A card shows the name, `user@address:port`, tags, the OS icon with a status badge, and CPU, memory and the root filesystem as bars, then the network rates, the load averages and the uptime. Every other filesystem over the disk threshold is named under the bars.
- The table has a row per host. **Columns** turns each column (status, CPU, memory, disk, network, load, uptime) on or off. Pointing at the disk cell lists the filesystems.
- Sort by the host list's order (**Host List Order**), name, status (problems first), CPU, memory, disk (the fullest filesystem), network or uptime. Values sort highest first.
- A value over its threshold shows in red. The thresholds are in Settings → General (90% by default; **Off** turns one off).
- A host that stops answering keeps its last readings, dimmed, with the time they were read.
- Click a card or row (or Tab to its name and press Enter) for the host's detail: CPU and network over the last 3 minutes, memory and swap, the filesystems, the total traffic of the counted interfaces (since each came up, usually at boot), the load and uptime. **Open Terminal** brings the host's tab to the front or opens one. Right-click a card or row for the same and **Edit**, which returns to the dashboard.
- The filesystems are those that store files, at most 32: not memory-backed or kernel ones, container layers, read-only images such as snap packages, or the files a container runtime mounts, and each disk and each ZFS pool once. They are read every fifth reading.

## How hosts are read

- Hosts are read only while the page is shown, the window is visible and the vault is unlocked. Leaving the page stops reading, and the connections close a minute later, so a look at a tab and back does not sign in again. Locking closes them at once.
- Each host is read every 3 seconds by default (**Refresh interval** in Settings → General), over the host's open connection (its terminal tabs' and port forwards') or else over a background SSH connection of its own. At most 8 connections are being set up at once, which keeps many hosts behind one jump host under OpenSSH's default `MaxStartups`. Until a jump host has let one connection through, the hosts behind it try it one at a time.
- A host whose first hop (the host, or its outermost jump host) does not answer the online probe is shown **Offline** and is not signed in to until it answers; it is checked again every minute, or at once with **Retry** in its menu.
- When a host's connection settings change (on another device, say), the dashboard connects again with the new ones and drops the readings made with the old ones.
- **Opening a tab** or starting a forward on a host the dashboard is connected to takes over that connection: a tab opens its shell without signing in again, then sets the environment variables, and the forwards set to start with the connection start. Once no tab and no forward is left on it, the connection closes and the dashboard connects again.
- Servers that do not run Linux show their online status only. The dashboard signs in once to find that out, and again only after the vault was locked or the host's connection settings changed.

Other labels: **Online**, **Waiting**, **Read over an open connection**, **Highlight CPU usage above**, **Highlight memory usage above**, **Highlight disk usage above**.

## Hosts that need the user

The background never shows a prompt. A host that cannot sign in on its own says why, with **Connect**, which opens a tab where the prompt can be answered. Once the tab is connected, the dashboard reads over it at once.

Other labels: **Asks for a password on every connection**, **The key’s passphrase isn’t saved**, **Asks for a password or verification code**, **Host key not confirmed yet**, **Host key changed**, **Sign-in failed**.

- A failed sign-in, with the server's answer, is not tried again on its own until the vault is locked and unlocked, so tools like fail2ban do not ban this computer. That includes a server that hangs up during sign-in, as one does after too many keys from the agent. **Retry** tries once more. Changing how the host connects (a new password, key, user, address or jump host, say) or confirming its host key in a tab also lets the dashboard try again; renaming it or marking it a favorite does not.
- When a jump host turns one host away (or turns the dashboard away when the jump host itself is on it), every host behind it waits too, so its rejected password is not tried once per host. **Retry** on any of them tries again from that host only; the others follow once it gets past the jump host.
- Other failures, such as a timeout or a lost connection, show the error and are tried again after a minute; **Retry** tries at once.

## Leaving hosts out

Right-click a card for **Leave Out of Dashboard**, or use the **…** menu of a group's section for **Leave Group Out of Dashboard**. A host left out is not shown or signed in to. The note at the bottom of the page counts the hosts left out; **Show** lists them dimmed, with **Include in Dashboard**, or **Include Group in Dashboard** for a host whose group is left out. The choice is kept on this device only and is not synced.

Other labels: **Left Out**.

## Not on the dashboard yet

History beyond the last few minutes, notifications, billing and traffic per billing cycle, temperatures, GPUs, Docker containers, custom commands per host, and a card for this computer. Readings stop when Hatoba is closed or the page is left: nothing is left running on the server.
