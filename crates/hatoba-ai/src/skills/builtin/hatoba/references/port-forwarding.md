# Port forwarding

Hatoba supports local port forwarding only, the equivalent of `ssh -L`. A port on this computer is forwarded through the SSH connection to an address that the server can reach. Remote forwarding (-R) and SOCKS / dynamic forwarding (-D) are not supported.

Forwards belong to a host. They are saved in the vault, sync like other items, and run only while a tab for that host is connected.

## Create or edit a rule

Open the host in the editor (**Edit** in the host's menu) and find the **Port Forwarding** section. A new host must be saved first ("Save the host to add port forwards."). Rules are saved the moment you add, change or delete them; the host form's **Save** button is not involved.

| en | zh-CN | ja |
|---|---|---|
| Port Forwarding | 端口转发 | ポートフォワーディング |
| Add Forward… | 添加转发… | 転送を追加… |
| Add Port Forward | 添加端口转发 | ポートフォワーディングを追加 |
| Edit Port Forward | 编辑端口转发 | ポートフォワーディングを編集 |
| Add | 添加 | 追加 |
| Local address | 本地地址 | ローカルアドレス |
| Port | 端口 | ポート |
| Destination | 目标地址 | 転送先 |
| Start when this host connects | 连接此主机时自动启动 | このホストに接続したときに開始 |
| Auto-start | 自动启动 | 自動起動 |
| auto | 自动 | 自動 |
| Edit | 编辑 | 編集 |
| Delete | 删除 | 削除 |
| Save | 保存 | 保存 |

- Each rule reads `local address:port → destination:port`. Use the pencil to edit and the bin to delete (with confirmation; with Cloud Sync on it is removed on other devices too).
- **Local address** is the address Hatoba listens on, and its port. `127.0.0.1` is reachable from this computer only. `0.0.0.0` exposes the port to the whole network, and the dialog warns about it. A local port of 0 picks a free port automatically (shown as **auto**).
- **Destination** is the host and port that the server connects to. `127.0.0.1` means the server itself. It can also be another machine the server can reach, such as an internal address.
- **Start when this host connects** (the **Auto-start** switch in the list) starts the rule as soon as the connection is up. Rules without it stay stopped until started by hand.

## Start and stop in a session

In a connected terminal tab, **Port Forwarding** in the status bar opens a popover with the host's rules. A badge on the button shows how many are running. The button is disabled until the tab is connected.

| en | zh-CN | ja |
|---|---|---|
| Start | 启动 | 開始 |
| Stop | 停止 | 停止 |
| Retry | 重试 | 再試行 |
| Manage Port Forwarding… | 管理端口转发… | ポートフォワーディングを管理… |
| This host has no port forwards yet. | 这台主机还没有端口转发。 | このホストにはポートフォワーディングがまだありません。 |
| Can’t read the port forwards. | 无法读取端口转发。 | ポートフォワーディングを読み込めません。 |

- Each row shows the state (stopped, running, failed), the local address and the destination. A running rule shows `localhost:<port>`; click it to copy the address (use it in a browser or client on this computer).
- **Start** starts a rule, **Stop** stops it, and **Retry** starts a failed one again. A failed rule shows the reason, for example that the local port is already in use.
- **Manage Port Forwarding…** goes to the host editor.
- Closing the tab or disconnecting stops its forwards.

## Typical use

To reach a database on the server's private network from this computer: add a rule with local address `127.0.0.1`, local port `5433`, destination `db.internal` port `5432`, and **Start when this host connects**. After connecting, point the client at `localhost:5433`.
