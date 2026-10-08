# SFTP file panel

The SFTP panel is a remote file browser beside a terminal. It uses the tab's SSH connection, so it needs a connected tab and a server whose SFTP subsystem is enabled.

## Open it

Click **SFTP** in the status bar of a connected terminal tab; click it again to hide the panel. Each tab has its own panel. It starts in the login (home) directory. Before the tab is connected the panel says "Connect to browse files."

| en | zh-CN | ja |
|---|---|---|
| SFTP | SFTP | SFTP |
| Show or hide the SFTP file panel | 显示或隐藏 SFTP 文件面板 | SFTP ファイルパネルを表示/非表示 |
| Connect to browse files. | 连接后才能浏览文件。 | 接続するとファイルを参照できます。 |

## Browsing

The header shows **SFTP**, the host name and three buttons. Under it, the path bar has a back arrow to the parent folder, the current path and the item count. The list shows folders first, then files, with Name, Size and Modified. Hover a row for permissions, exact size and modification time. Files that only the owner can read (private keys, `.env`) get a lock icon.

| en | zh-CN | ja |
|---|---|---|
| Upload files | 上传文件 | ファイルをアップロード |
| New folder | 新建文件夹 | 新規フォルダ |
| Refresh | 刷新 | 更新 |
| Parent folder | 上级目录 | 上のフォルダ |
| Name | 名称 | 名前 |
| Size | 大小 | サイズ |
| Modified | 修改 | 更新 |
| This folder is empty | 此文件夹为空 | このフォルダは空です |
| Loading… | 正在读取… | 読み込み中… |
| Can’t read this folder | 无法读取此目录 | このフォルダを読み取れません |

- Double-click a folder (or select it and press Enter) to open it. Backspace goes to the parent folder. The arrow keys move the selection.
- **Can’t read this folder** (with Retry) means the server refused the listing, for example because of permissions.

## Files and folders

Right-click a file or folder, or the empty area of the list, for the menu. Keyboard: Enter opens, F2 renames, Delete deletes.

| en | zh-CN | ja |
|---|---|---|
| Open | 打开 | 開く |
| Download… | 下载… | ダウンロード… |
| Rename | 重命名 | 名前を変更 |
| Delete… | 删除… | 削除… |
| Copy Path | 复制路径 | パスをコピー |
| New Folder | 新建文件夹 | 新規フォルダ |
| Upload Files… | 上传文件… | ファイルをアップロード… |

- **Download…** is for files: it asks where to save the file locally. Folders cannot be downloaded.
- **Upload Files…** (or the upload button) picks local files, and dropping files from the desktop onto the panel uploads them to the folder that is open ("Drop to upload to …"). Only files are uploaded, not folders. An upload to an existing name replaces the file.
- **New Folder** adds a row where you type the name. **Rename** edits the name in place; Enter or leaving the field commits and Esc cancels. A name cannot be empty or contain `/`.
- **Delete…** asks first. A file is deleted for good; a folder is deleted with everything in it ("permanently deleted from the server. This can’t be undone"). The root directory cannot be deleted.
- **Copy Path** copies the full remote path to the clipboard.

| en | zh-CN | ja |
|---|---|---|
| The name can’t be empty or contain /. | 名称不能为空，也不能包含 /。 | 名前を空にしたり、/ を含めたりすることはできません。 |
| This file will be permanently deleted from the server. This can’t be undone. | 这个文件会从服务器上永久删除，无法撤销。 | このファイルはサーバーから完全に削除されます。元に戻せません。 |
| This folder and everything in it will be permanently deleted from the server. This can’t be undone. | 这个文件夹及其中的所有内容会从服务器上永久删除，无法撤销。 | このフォルダとその中身はすべてサーバーから完全に削除されます。元に戻せません。 |
| Path copied | 路径已复制 | パスをコピーしました |

## Transfers

Uploads and downloads show in a **Transfers** section at the bottom of the panel, one row each, with a progress bar, percentage, speed and time left. The × cancels a running transfer, or dismisses a finished one (**Cancel transfer** and **Dismiss**). A download or upload can fail midway; the row then shows the error in red. The list refreshes after an upload finishes.

| en | zh-CN | ja |
|---|---|---|
| Transfers | 传输 | 転送 |
| Cancel transfer | 取消传输 | 転送をキャンセル |
| Dismiss | 清除 | 消去 |
| Cancelled | 已取消 | キャンセルしました |
| Done | 已完成 | 完了 |

## Not supported

Editing a remote file in the app, uploading or downloading folders, moving files by drag and drop inside the panel, and changing permissions. For those, use commands in the terminal (`scp`, `rsync`, `chmod`) or an editor on the server.

Errors such as "The SFTP subsystem isn’t available." are covered in `references/troubleshooting.md`.
