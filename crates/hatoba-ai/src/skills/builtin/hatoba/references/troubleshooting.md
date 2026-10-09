# Troubleshooting: keys, vault, files and other messages

The messages are shown exactly as the app prints them in each language, so a message the user pastes can be matched to a row. Failed SSH connections, jump hosts, host key prompts and dropped connections are in `references/troubleshooting-connections.md`. Sync and Cloudflare errors are in `references/troubleshooting-sync.md`, AI errors in `references/troubleshooting-ai.md`.

## Key errors

When importing a key, or connecting with one:

| en | zh-CN | ja |
|---|---|---|
| Unsupported private key format. Use OpenSSH, PEM or PuTTY (.ppk). | 不支持的私钥格式。请使用 OpenSSH、PEM 或 PuTTY (.ppk) 格式。 | サポートされていない秘密鍵の形式です。OpenSSH、PEM、PuTTY (.ppk) を使用してください。 |
| This key is protected by a passphrase. Enter it to continue. | 这个私钥受口令保护，请输入口令。 | この鍵はパスフレーズで保護されています。パスフレーズを入力してください。 |
| Incorrect passphrase for this key. | 私钥口令不正确。 | 鍵のパスフレーズが正しくありません。 |
| Unsupported key algorithm. Ed25519, ECDSA and RSA are supported. | 不支持这种密钥算法。支持 Ed25519、ECDSA 和 RSA。 | サポートされていない鍵アルゴリズムです。Ed25519、ECDSA、RSA に対応しています。 |
| The private key is invalid or damaged. | 私钥内容无效或已损坏。 | 秘密鍵が無効か、破損しています。 |
| This private key can’t be read. | 无法读取这个私钥。 | この秘密鍵を読み込めません。 |
| Ask Each Time | 每次询问 | 毎回入力 |

- Supported formats are OpenSSH, PEM and PuTTY (.ppk); supported algorithms are Ed25519, ECDSA and RSA (for example, DSA keys are not).
- Enter the passphrase in the import form (it is saved encrypted with the key) or when Hatoba asks for it at connect; a wrong one asks again. A key on a jump host needs its passphrase saved.
- A key that is "invalid or damaged" is usually truncated: copy the whole text including the BEGIN and END lines, or import the file instead of pasting.
- Deleting a key switches the hosts that used it to **Ask Each Time**.

## Locked, password and recovery

| en | zh-CN | ja |
|---|---|---|
| The vault is locked. | 保险库已锁定。 | 保管庫はロックされています。 |
| Incorrect master password. | 主密码不正确。 | マスターパスワードが正しくありません。 |
| That recovery code isn’t right. Check it and try again. | 恢复码不正确，请检查后重试。 | リカバリーコードが正しくありません。確認してもう一度お試しください。 |
| The two passwords don’t match. | 两次输入的主密码不一致。 | 2 つのパスワードが一致しません。 |
| The master password needs at least 8 characters. | 主密码至少需要 8 个字符。 | マスターパスワードは 8 文字以上にしてください。 |
| That password is too easy to guess. Choose a longer or less common one. | 这个主密码太容易被猜到，请换一个更长或更不常见的。 | このパスワードは推測されやすすぎます。より長いもの、または一般的でないものにしてください。 |
| No vault has been created yet. | 尚未创建保险库。 | 保管庫がまだ作成されていません。 |
| A vault already exists. | 保险库已存在。 | 保管庫はすでに存在します。 |

- After wrong master passwords, "Too many attempts. Try again in N s." appears and the password field is disabled until the countdown ends. Wait; each further miss lengthens the delay.
- A forgotten master password can be reset only with the recovery code (**Forgot it? Use your recovery code**). With neither, the data cannot be recovered, and Hatoba cannot reset it.
- The recovery code ignores case, dashes and spaces. A new code can be generated in Settings → Security (**Generate New Code…**), after which the old one stops working.

| en | zh-CN | ja |
|---|---|---|
| Forgot it? Use your recovery code | 忘记主密码？使用恢复码 | マスターパスワードをお忘れですか？ リカバリーコードを使う |
| Generate New Code… | 生成新的恢复码… | 新しいコードを生成… |

## Files, forwards and other messages

| en | zh-CN | ja |
|---|---|---|
| The file operation failed. | 文件操作失败。 | ファイル操作に失敗しました。 |
| Couldn’t read or write the file. | 读写文件失败。 | ファイルの読み書きに失敗しました。 |
| Something went wrong. | 发生了意外错误。 | 予期しないエラーが発生しました。 |
| This item no longer exists. It may have been deleted on another device. | 找不到该项目，它可能已在其他设备上被删除。 | この項目は存在しません。別のデバイスで削除された可能性があります。 |
| Cancelled. | 已取消。 | キャンセルしました。 |
| Can’t access the clipboard. | 无法访问剪贴板。 | クリップボードにアクセスできません。 |
| Can’t read this folder | 无法读取此目录 | このフォルダを読み取れません |
| The name can’t be empty or contain /. | 名称不能为空，也不能包含 /。 | 名前を空にしたり、/ を含めたりすることはできません。 |

- "The file operation failed." (SFTP panel) means the server refused the operation: permissions, a name that already exists, or a full disk. See `references/sftp.md`. Folders cannot be uploaded or downloaded.
- "Couldn’t read or write the file." is a local problem, such as a path that cannot be written when saving a download, a backup, or a skill export.
- "This item no longer exists." usually means it was deleted on another device; refresh the list or recreate it.
- "Something went wrong." is a generic failure. Retry once, then use the feedback path below.
- Port forwarding failures show in the **Port Forwarding** popover; a local port that is already in use is the usual cause (edit the rule, or use local port 0). See `references/port-forwarding.md`.
- On Windows the logs are in `%LOCALAPPDATA%\app.hatoba.desktop\logs`.

| en | zh-CN | ja |
|---|---|---|
| Port Forwarding | 端口转发 | ポートフォワーディング |

## Still not solved

If a problem is not covered by the troubleshooting files and looks like a bug, or the user wants something Hatoba lacks, use the feedback path in SKILL.md ("Project, feedback and newer information"): **Report a Problem** in Settings → About opens GitHub's bug report form, and feature requests go to https://github.com/scarletkc/Hatoba/issues/new?template=feature_request.yml. Offer to draft the text, and remind the user to leave out host names, addresses, keys, passwords and other secrets. The user submits it.

| en | zh-CN | ja |
|---|---|---|
| Report a Problem | 反馈问题 | 問題を報告 |
| About | 关于 | 情報 |
