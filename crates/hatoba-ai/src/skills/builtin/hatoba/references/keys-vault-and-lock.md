# Keys, vault and lock

## The vault and the master password

Hosts, SSH keys, saved passwords, known host fingerprints, forwards, AI settings, skills and settings live in an encrypted vault on the device. Each item is encrypted with AES-256-GCM using a random vault key, and the vault key is encrypted with a key derived from the master password (Argon2id). The master password never leaves the device. Hatoba does not store it and cannot reset it.

On first launch the window offers two ways to start:

| en | zh-CN | ja |
|---|---|---|
| Create a New Vault | 创建新保险库 | 新しい保管庫を作成 |
| Restore from Cloud | 从云端恢复 | クラウドから復元 |
| Start | 开始 | 開始 |
| Password | 主密码 | パスワード |
| Recovery | 恢复码 | リカバリー |
| Connect | 连接 | 接続 |

- **Create a New Vault** sets a master password (at least 8 characters, with a strength hint: Weak, Fair, Strong, Very strong; a password that is too easy to guess is refused) and then shows a recovery code.
- **Restore from Cloud** is for a new device that should join an existing sync setup. See `references/sync.md`.
- The recovery code is shown once. **Save as Text** or **Print** it, then type its last group to confirm; **Finish Setup** completes the setup. It is the only way back in if the master password is forgotten. A new code can be generated in Settings → Security (**Generate New Code…**), and then the old one stops working.

| en | zh-CN | ja |
|---|---|---|
| Key | 密钥 | 鍵 |
| Ask Each Time | 每次询问 | 毎回入力 |
| Save Your Recovery Code | 保存你的恢复码 | リカバリーコードを保存 |
| Save as Text | 存储为文本 | テキストで保存 |
| Print | 打印 | 印刷 |
| Finish Setup | 完成设置 | セットアップを完了 |
| Recovery Code | 恢复码 | リカバリーコード |
| Generate New Code… | 生成新的恢复码… | 新しいコードを生成… |
| Weak | 弱 | 弱い |
| Fair | 一般 | 普通 |
| Strong | 强 | 強い |
| Very strong | 很强 | とても強い |

## Locking and unlocking

Lock with the lock button at the bottom of the sidebar, Ctrl+Shift+L (⌘L on macOS), or Settings → Security → **Lock Now**. Hatoba also locks after the idle time set in **Auto-Lock**, and whenever the computer goes to sleep.

- Auto-Lock offers 1, 5, 15 (the default), 30 minutes, 1 hour, or Never. It is a synced setting.
- Open SSH sessions stay connected behind the lock screen ("N sessions stay connected in the background"), unless **Disconnect all sessions when locked** is on. The AI assistant stops working and MCP servers stop when the vault locks.
- The lock screen asks for the master password (**Unlock**). After wrong attempts the field is disabled for a growing delay, with a countdown such as "Too many attempts. Try again in 30 s."
- **Forgot it? Use your recovery code** opens **Reset with Recovery Code**: enter the recovery code (case and dashes do not matter) and a new master password, then **Reset and Unlock**. It works offline and loses no data.
- **Unlock with Windows Hello** is a switch in Settings → Security. It appears only on Windows PCs where Windows Hello is set up, and asks for the master password once to turn on. The lock screen then shows a Windows Hello button. Touch ID is not available yet.

| en | zh-CN | ja |
|---|---|---|
| Auto-Lock | 自动锁定 | 自動ロック |
| Disconnect all sessions when locked | 锁定时断开所有会话 | ロック時にすべてのセッションを切断 |
| Lock Now | 立即锁定 | 今すぐロック |
| Lock | 锁定 | ロック |
| Never | 从不 | しない |
| 1 hour | 1 小时 | 1 時間 |
| Hatoba is locked | Hatoba 已锁定 | Hatoba はロックされています |
| Master password | 主密码 | マスターパスワード |
| Unlock | 解锁 | ロック解除 |
| Forgot it? Use your recovery code | 忘记主密码？使用恢复码 | マスターパスワードをお忘れですか？ リカバリーコードを使う |
| Reset with Recovery Code | 使用恢复码重置主密码 | リカバリーコードでリセット |
| New Master Password | 新主密码 | 新しいマスターパスワード |
| Reset and Unlock | 重置并解锁 | リセットしてロック解除 |
| Unlock with Windows Hello | 使用 Windows Hello 解锁 | Windows Hello でロック解除 |
| Security | 安全 | セキュリティ |

## Master password, backup and clipboard

- Settings → Security → **Master Password** → **Change…** re-encrypts only the vault key; saved hosts and keys are untouched. With Cloud Sync on, the other devices are signed out and need the new password. The Cloud Sync page has the same action.
- Settings → General → **Export Encrypted Backup** → **Export…** saves the vault as one `hatoba-backup-YYYY-MM-DD.hatoba` file that only the master password opens. The app has no screen to import a backup file.
- **Copy Password** on a host clears the clipboard 30 seconds after copying.

| en | zh-CN | ja |
|---|---|---|
| Master Password | 主密码 | マスターパスワード |
| Change… | 更改主密码… | 変更… |
| Change Master Password | 更改主密码 | マスターパスワードを変更 |
| Export Encrypted Backup | 导出加密备份 | 暗号化バックアップをエクスポート |
| Export… | 导出… | エクスポート… |
| General | 通用 | 一般 |
| Copy Password | 复制密码 | パスワードをコピー |

## The Keys page

Open it from the sidebar (**Keys**, under **Vault**). It lists the SSH keys in the vault, searchable (**Search keys**), with the buttons **Import…** and **Generate Key**. Columns are Name, Type, Fingerprint, Created and Used By (the first host using the key, plus how many more, or **Unused**). Selecting a key opens a detail panel with its fingerprint, creation date, last use, the public key (**Copy**), the hosts that use it, and the actions **Rename**, **Deploy to Host…** and **Delete…**.

| en | zh-CN | ja |
|---|---|---|
| Vault | 保险库 | 保管庫 |
| Keys | 密钥库 | 鍵 |
| Search keys | 搜索密钥 | 鍵を検索 |
| Import… | 导入… | 読み込み… |
| Generate Key | 生成新密钥 | 新しい鍵を生成 |
| Name | 名称 | 名前 |
| Type | 类型 | 種類 |
| Fingerprint | 指纹 | フィンガープリント |
| Created | 创建 | 作成日 |
| Used By | 使用中 | 使用中 |
| Unused | 未使用 | 未使用 |
| Public Key | 公钥 | 公開鍵 |
| Private Key | 私钥 | 秘密鍵 |
| Used by | 使用中的主机 | 使用中のホスト |
| Last used | 上次使用 | 最終使用 |
| Copy public key | 复制公钥 | 公開鍵をコピー |
| Copy | 复制 | コピー |
| Rename | 重命名 | 名前を変更 |
| Deploy to Host… | 部署到主机… | ホストに配置… |
| Delete… | 删除… | 削除… |

- **Import…** opens **Import Private Key**. Choose a file (**Choose File**) or **Paste Text**, give the key a name, and enter its passphrase if it has one (the passphrase is saved encrypted with the key). OpenSSH, PEM and PuTTY (.ppk) formats are supported; the algorithms are Ed25519, ECDSA and RSA.
- **Generate Key** makes an Ed25519 key (recommended) or an RSA 4096 key on this device, with a comment and an optional passphrase. RSA takes a few seconds.
- **Deploy to Host…** picks a host, connects with that host's existing authentication, and appends the key's public key to `~/.ssh/authorized_keys` on the server. After that the host can use **Key** authentication.
- **Delete…** removes the key from the vault for good, on all synced devices. Hosts that used it fall back to **Ask Each Time**.
- The private key cannot be viewed or exported; only the public key can be copied. It is decrypted in memory only while connecting.

| en | zh-CN | ja |
|---|---|---|
| Import Private Key | 导入私钥 | 秘密鍵を読み込む |
| Choose File | 选择文件 | ファイルを選択 |
| Paste Text | 粘贴文本 | テキストを貼り付け |
| Choose File… | 选择文件… | ファイルを選択… |
| Passphrase (optional) | 口令（可选） | パスフレーズ（任意） |
| Ed25519 | Ed25519 | Ed25519 |
| RSA 4096 | RSA 4096 | RSA 4096 |
| Comment | 注释 | コメント |
| Confirm Passphrase | 确认口令 | パスフレーズ（確認） |
| Rename Key | 重命名密钥 | 鍵の名前を変更 |
| Deploy Public Key to Host | 部署公钥到主机 | 公開鍵をホストに配置 |
| Host | 主机 | ホスト |
| Deploy | 部署 | 配置 |
| Delete Key | 删除密钥 | 鍵を削除 |
| Import | 导入 | 読み込む |
| Generate | 生成 | 生成 |

For key errors (unsupported format, wrong passphrase) see `references/troubleshooting.md`.
