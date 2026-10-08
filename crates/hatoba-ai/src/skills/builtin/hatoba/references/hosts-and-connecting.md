# Hosts and connecting

## The host list

The home tab shows the host list for the sidebar item that is selected: **All Hosts**, **Favorites**, **Recent** (hosts that have been connected before), a group, or a tag. A host is one saved SSH destination. Groups are flat (one level) and a host belongs to at most one group; tags are free text and a host can have several.

- The header has a search box (fuzzy match on name, address, user name and tags; Ctrl+Shift+K or ⌘K focuses it), the sort button, and **New Host**.
- Each row shows the name, `user@address:port`, the jump host if any, tags, and the time of the last connection. The dot is green when the host is connected or answers a TCP probe, grey otherwise. A host whose latest attempt in this launch failed shows **Failed** in red.
- To connect: double-click a row, select it and press Enter, or click **Connect**. Up and down move the selection, and typing starts a search. The same host can be opened in several tabs.
- Right-click a row for its menu.

| en | zh-CN | ja |
|---|---|---|
| All Hosts | 全部主机 | すべてのホスト |
| Favorites | 收藏 | お気に入り |
| Recent | 最近连接 | 最近の接続 |
| Search name, IP or tag | 搜索名称、IP 或标签 | 名前、IP、タグを検索 |
| Sort by | 排序方式 | 並べ替え |
| Name | 名称 | 名前 |
| Address | 地址 | アドレス |
| New Host | 新建主机 | 新規ホスト |
| Tags | 标签 | タグ |
| Last Connected | 最近连接 | 最終接続 |
| Connect | 连接 | 接続 |
| Failed | 连接失败 | 接続失敗 |
| Edit | 编辑 | 編集 |
| Duplicate | 复制主机 | 複製 |
| Add to Favorites | 加入收藏 | お気に入りに追加 |
| Remove from Favorites | 取消收藏 | お気に入りから削除 |
| Copy Address | 复制地址 | アドレスをコピー |
| Copy Password | 复制密码 | パスワードをコピー |
| Delete… | 删除… | 削除… |

- **Copy Password** appears only for hosts with a saved password. It copies the password and clears the clipboard after 30 seconds.
- **Delete…** asks for confirmation. With Cloud Sync on, the host is removed on the other devices too.
- A group's right-click menu in the sidebar has **Rename Group** and **Delete Group**. Deleting a group moves its hosts to All Hosts; it does not delete them.
- The reachability dots can be turned off in Settings → General (**Show host reachability**). When on, Hatoba tries a TCP connection to the listed hosts every 60 seconds, with no login attempt.

| en | zh-CN | ja |
|---|---|---|
| Rename Group | 重命名分组 | グループ名を変更 |
| Delete Group | 删除分组 | グループを削除 |
| General | 通用 | 一般 |
| Show host reachability | 显示主机在线状态 | ホストの接続状態を表示 |

## Import from ~/.ssh/config

When the vault has no hosts yet, the empty host list offers **Import SSH Config**. It reads `~/.ssh/config` (on Windows `%USERPROFILE%\.ssh\config`), lists the `Host` entries, and imports the ones you tick. Entries whose name already exists are unticked and marked **Already exists**. A ProxyJump is kept when its first hop names a host that exists in Hatoba (otherwise the import warns); ProxyCommand is not supported. The button is not shown once the vault has hosts, so import before adding hosts, or add hosts by hand. Importing PuTTY sessions is not supported.

| en | zh-CN | ja |
|---|---|---|
| Import SSH Config | 从 SSH 配置导入 | SSH 設定からインポート |
| Select All | 全选 | すべて選択 |
| Select None | 全不选 | 選択解除 |
| Already exists | 已存在 | 登録済み |

## The host editor

**New Host**, **Edit** in the row menu, or **Edit Host** (in a terminal tab's **More actions** menu, or on the connection-failed card) opens the editor on the home tab. **Save** (or Ctrl+S, or Enter in a text field) saves, **Cancel** or the back arrow leaves, and Esc leaves an untouched form. The star in the header toggles the favorite. Name and address are required, and the port must be 1 to 65535.

| en | zh-CN | ja |
|---|---|---|
| Edit Host | 编辑主机 | ホストを編集 |
| General | 基本信息 | 基本情報 |
| Port | 端口 | ポート |
| Username | 用户名 | ユーザー名 |
| Authentication | 认证 | 認証 |
| Method | 方式 | 方式 |
| Organization & Network | 组织与网络 | 整理とネットワーク |
| Group | 分组 | グループ |
| Jump Host | 跳板机 | 踏み台 |
| Port Forwarding | 端口转发 | ポートフォワーディング |
| Notes | 备注 | メモ |
| Test Connection | 测试连接 | 接続テスト |
| Delete Host | 删除主机 | ホストを削除 |
| Save | 保存 | 保存 |
| Cancel | 取消 | キャンセル |

- **Test Connection** connects with the current form values (saved or not), authenticates, checks the host key (you may be asked to trust it), shows the latency, and disconnects. It does not save the host.
- **Delete Host** is at the bottom of the editor for an existing host.
- **Port Forwarding** in the editor lists the host's forwards; see `references/port-forwarding.md`. A new host must be saved before forwards can be added.

## Authentication methods

**Method** chooses how Hatoba signs in:

| en | zh-CN | ja |
|---|---|---|
| Password | 密码 | パスワード |
| Key | 密钥 | 鍵 |
| Ask Each Time | 每次询问 | 毎回入力 |
| SSH Agent | SSH Agent | SSH Agent |
| Saved | 已保存 | 保存済み |
| Replace | 替换 | 置き換え |
| Keep Saved Password | 保留已保存的密码 | 保存済みのパスワードを使う |
| Choose a key… | 选择密钥… | 鍵を選択… |
| Manage Keys | 管理密钥库 | 鍵を管理 |

- **Password**: stored encrypted in the vault. After saving, the field shows **Saved** and the password can be replaced (**Replace**, **Keep Saved Password**) but never viewed.
- **Key**: choose a key from the Keys page (**Choose a key…**; **Manage Keys** opens the Keys page). RSA, Ed25519 and ECDSA keys work. If the key has a passphrase that was saved with it, it is used; otherwise Hatoba asks for the passphrase when connecting (**Enter the key passphrase**), and does not save it.
- **Ask Each Time**: Hatoba asks for the password at every connection and never saves it.
- **SSH Agent**: uses the keys in the system SSH agent. On Windows that is the OpenSSH Authentication Agent service (it must be running); on macOS and Linux it is the agent that `SSH_AUTH_SOCK` points to. Pageant is not supported.
- Servers that ask for a second factor (keyboard-interactive, such as a one-time code) show a dialog titled **Additional verification required** with a **Verify** button. This needs no setting.
- Deleting a key switches the hosts that used it to **Ask Each Time**.

## Jump host

**Jump Host** (ProxyJump) routes the connection through another saved host. Pick it from the list; the list excludes the host itself, and a loop of jump hosts is rejected. A jump host can have its own jump host. Each hop uses its own saved settings, and a jump host must use a saved password, a key with its passphrase saved, or **SSH Agent**. HTTP and SOCKS proxies are not supported. The status bar of the tab shows "via" and the jump host's name.

## Connecting, host keys and prompts

A connection opens a new terminal tab (orange dot while connecting). The connection attempt times out after 15 seconds. Dialogs that can appear:

| en | zh-CN | ja |
|---|---|---|
| Fingerprint | 指纹 | フィンガープリント |
| Trust and Connect | 信任并连接 | 信頼して接続 |
| Host key has changed | 主机指纹已改变 | ホストキーが変更されました |
| Trusted fingerprint | 已信任的指纹 | 信頼済みのフィンガープリント |
| Fingerprint received now | 现在收到的指纹 | 今回受信したフィンガープリント |
| Disconnect | 断开连接 | 切断 |
| Update Fingerprint and Connect | 更新指纹并连接 | フィンガープリントを更新して接続 |
| Additional verification required | 需要额外验证 | 追加の認証が必要です |
| Verify | 验证 | 認証 |
| Enter the key passphrase | 输入私钥口令 | 秘密鍵のパスフレーズを入力 |
| Key passphrase | 私钥口令 | 鍵のパスフレーズ |

- First connection to a server: a dialog shows the key type and fingerprint. Compare it with what the server's administrator gave you, then **Trust and Connect**. The fingerprint is saved to the vault and syncs to other devices. **Cancel** stops the connection.
- Changed key: **Host key has changed** blocks the connection and shows the trusted and the received fingerprints. **Disconnect** is the default. **Update Fingerprint and Connect** replaces the saved fingerprint; use it only when you know the server's key was replaced. There is no screen to list or remove saved fingerprints.
- **Ask Each Time** hosts show a password prompt; **Connect** continues. A wrong key passphrase asks again.

## When a connection fails or ends

- A failed attempt shows a card over the terminal titled "Can’t connect to" plus the host name. It gives the reason, a short code (for example ETIMEDOUT) with the time, and the buttons **Copy Diagnostics**, **Ask AI** (opens the AI panel with the diagnostics attached; nothing is sent until the user sends it, see `references/ai-attachments.md`), **Edit Host** and **Retry**. Diagnostics hold the host, user, auth method, error and time, and no secrets. See `references/troubleshooting-connections.md` for each reason.
- A connection that ends later (the server closed it, the network dropped, keepalives went unanswered) shows the banner **Connection closed** with **Reconnect**. Hatoba never reconnects by itself. **More actions** has Reconnect and Disconnect too.
- Closing a tab (the × on the tab, middle-click, or Ctrl+Shift+W / ⌘W) ends its session.
- Locking keeps open sessions connected in the background unless **Disconnect all sessions when locked** is on (Settings → Security).

| en | zh-CN | ja |
|---|---|---|
| Copy Diagnostics | 复制诊断信息 | 診断情報をコピー |
| Ask AI | 询问 AI | AI に質問 |
| Retry | 重试 | 再試行 |
| Connection closed | 连接已断开 | 接続が切断されました |
| Reconnect | 重新连接 | 再接続 |
| More actions | 更多操作 | その他の操作 |
| Disconnect all sessions when locked | 锁定时断开所有会话 | ロック時にすべてのセッションを切断 |
| Security | 安全 | セキュリティ |
