# Dashboard

**Dashboard** at the top of the sidebar shows the online status and resource usage of every saved host on one page, without opening a tab on each. It reads Linux servers over SSH, as the terminal's resource usage does (see `references/terminal.md`): nothing is installed on the server and no Hatoba server is involved. The readings stay in memory on this device; they are never stored or synced.

| en | zh-CN | ja |
|---|---|---|
| Dashboard | 仪表盘 | ダッシュボード |
| Cards | 卡片 | カード |
| Table | 表格 | 表 |
| Columns | 列 | 列 |
| Host List Order | 主机列表顺序 | ホスト一覧の順序 |
| No Group | 未分组 | グループなし |
| Open Terminal | 打开终端 | ターミナルを開く |
| Filesystems | 文件系统 | ファイルシステム |

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
- Each host is read every 3 seconds by default (**Refresh interval** in Settings → General), over a connected tab's connection or else over a background SSH connection of its own. At most 8 connections are being set up at once, which keeps many hosts behind one jump host under OpenSSH's default `MaxStartups`. Until a jump host has let one connection through, the hosts behind it try it one at a time.
- A host whose first hop (the host, or its outermost jump host) does not answer the online probe is shown **Offline** and is not signed in to until it answers; it is checked again every minute, or at once with **Retry** in its menu.
- When a host's connection settings change (on another device, say), the dashboard connects again with the new ones and drops the readings made with the old ones.
- **Opening a tab** on a host the dashboard is connected to takes over that connection: the tab opens its shell without signing in again, then sets the environment variables and starts the forwards set to start with the connection. Closing the tab closes the connection, and the dashboard connects again.
- Servers that do not run Linux show their online status only. The dashboard signs in once to find that out, and again only after the vault was locked or the host's connection settings changed.

| en | zh-CN | ja |
|---|---|---|
| Online | 在线 | オンライン |
| Offline | 离线 | オフライン |
| Waiting | 等待中 | 待機中 |
| Read over an open terminal | 通过已打开的终端读取 | 開いているターミナル経由で読み取り |
| Refresh interval | 刷新间隔 | 更新間隔 |
| Highlight CPU usage above | CPU 占用高亮阈值 | CPU 使用率の強調しきい値 |
| Highlight memory usage above | 内存占用高亮阈值 | メモリ使用率の強調しきい値 |
| Highlight disk usage above | 磁盘占用高亮阈值 | ディスク使用率の強調しきい値 |
| Off | 关闭 | オフ |

## Hosts that need the user

The background never shows a prompt. A host that cannot sign in on its own says why, with **Connect**, which opens a tab where the prompt can be answered. Once the tab is connected, the dashboard reads over it at once.

| en | zh-CN | ja |
|---|---|---|
| Asks for a password on every connection | 每次连接都询问密码 | 接続のたびにパスワードを確認します |
| The key’s passphrase isn’t saved | 密钥口令未保存 | 鍵のパスフレーズが保存されていません |
| Asks for a password or verification code | 需要输入密码或验证码 | パスワードまたは確認コードの入力が必要です |
| Host key not confirmed yet | 主机指纹尚未确认 | ホスト鍵がまだ確認されていません |
| Host key changed | 主机指纹已变更 | ホスト鍵が変更されています |
| Sign-in failed | 登录失败 | サインインに失敗しました |
| Connect | 连接 | 接続 |
| Retry | 重试 | 再試行 |

- A failed sign-in, with the server's answer, is not tried again on its own until the vault is locked and unlocked, so tools like fail2ban do not ban this computer. That includes a server that hangs up during sign-in, as one does after too many keys from the agent. **Retry** tries once more. Changing how the host connects (a new password, key, user, address or jump host, say) or confirming its host key in a tab also lets the dashboard try again; renaming it or marking it a favorite does not.
- When a jump host turns one host away (or turns the dashboard away when the jump host itself is on it), every host behind it waits too, so its rejected password is not tried once per host. **Retry** on any of them tries again from that host only; the others follow once it gets past the jump host.
- Other failures, such as a timeout or a lost connection, show the error and are tried again after a minute; **Retry** tries at once.

## Leaving hosts out

Right-click a card for **Leave Out of Dashboard**, or use the **…** menu of a group's section for **Leave Group Out of Dashboard**. A host left out is not shown or signed in to. The note at the bottom of the page counts the hosts left out; **Show** lists them dimmed, with **Include in Dashboard**, or **Include Group in Dashboard** for a host whose group is left out. The choice is kept on this device only and is not synced.

| en | zh-CN | ja |
|---|---|---|
| Leave Out of Dashboard | 不在仪表盘中显示 | ダッシュボードから除外 |
| Include in Dashboard | 在仪表盘中显示 | ダッシュボードに含める |
| Leave Group Out of Dashboard | 不在仪表盘中显示此分组 | グループをダッシュボードから除外 |
| Include Group in Dashboard | 在仪表盘中显示此分组 | グループをダッシュボードに含める |
| Show | 显示 | 表示 |
| Left Out | 未纳入 | 除外中 |

## Not on the dashboard yet

History beyond the last few minutes, notifications, billing and traffic per billing cycle, temperatures, GPUs, Docker containers, custom commands per host, and a card for this computer. Readings stop when Hatoba is closed or the page is left: nothing is left running on the server.
