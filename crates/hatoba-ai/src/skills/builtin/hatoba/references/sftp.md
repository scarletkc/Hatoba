# SFTP file panel

The SFTP panel is a remote file browser beside a terminal. It uses the tab's SSH connection, so it needs a connected tab and a server whose SFTP subsystem is enabled.

## Open it

Click **SFTP** in the status bar of a connected terminal tab; click it again to hide the panel. Each tab has its own panel. It starts in the login (home) directory. Before the tab is connected the panel says "Connect to browse files."

Other labels: **Show or hide the SFTP file panel**.

## Browsing

The header shows **SFTP**, the host name and three buttons. Under it, the path bar has a back arrow to the parent folder, the current path and the item count. The list shows folders first, then files, with Name, Size and Modified. Hover a row for permissions, exact size and modification time. Files that only the owner can read (private keys, `.env`) get a lock icon.

Other labels: **Upload files**, **New folder**, **Refresh**, **Parent folder**, **This folder is empty**, **Loading…**.

- Double-click a folder (or select it and press Enter) to open it. Backspace goes to the parent folder. The arrow keys move the selection.
- **Can’t read this folder** (with Retry) means the server refused the listing, for example because of permissions.

## Files and folders

Right-click a file or folder, or the empty area of the list, for the menu. Keyboard: Enter opens, F2 renames, Delete deletes.

- **Download** is for files: it asks where to save the file locally. Folders cannot be downloaded.
- **Edit** is for files: it opens the file in an editor on this computer and uploads every save; see "Edit files" below.
- **Upload Files** (or the upload button) picks local files, and dropping files from the desktop onto the panel uploads them to the folder that is open ("Drop to upload to …"). Only files are uploaded, not folders. An upload to an existing name replaces the file.
- **New Folder** adds a row where you type the name. **Rename** edits the name in place; Enter or leaving the field commits and Esc cancels. A name cannot be empty or contain `/`.
- **Delete** asks first. A file is deleted for good; a folder is deleted with everything in it ("permanently deleted from the server. This can’t be undone"). The root directory cannot be deleted.
- **Copy Path** copies the full remote path to the clipboard.

Other labels: **Path copied**.

Other messages:

- "The name can’t be empty or contain /."
- "This file will be permanently deleted from the server. This can’t be undone."
- "This folder and everything in it will be permanently deleted from the server. This can’t be undone."

## Edit files

**Edit** in a file's menu opens the file in an editor on this computer. Hatoba downloads it into a folder of its own (under the app's data folder, keeping the file name so the editor picks the syntax by its extension), opens it, and uploads every save over the tab's connection, once the file has not changed for half a second. Each upload shows under **Transfers**. Double-click still downloads.

- **The editor** is set in Settings → Terminal → **SFTP** → **Editor**, on this device only (not synced): **System Default** opens each file with the app the system opens that kind of file with, and **Choose a Program…** picks a program (an `.exe` on Windows, an app on macOS). A file the system has no app for, such as `nginx.conf` or `.bashrc`, opens in the platform's text editor: Notepad on Windows, TextEdit (or the default text editor) on macOS, the desktop's text editor on Linux. Scripts, programs and web pages (`.sh`, `.py`, `.bat`, `.ps1`, `.js`, `.exe`, `.html`, `.svg` and similar) always open in the text editor rather than in the app that would run them, such as a browser.
- **Editing** above the transfers lists the tab's files with their state: **Saved** (the server has the last save), **Waiting to upload**, **Uploading…**, **Changed on the server**, or the server's error in red. Click a file's name to open it in the editor again (**Open in Editor**). **Upload Again** retries a failed upload, and × is **Stop Editing**.
- **A change on the server**: before each upload Hatoba compares the server's file with what it last downloaded or uploaded (size and modification time). When it changed, it asks "“name” changed on the server": **Overwrite** uploads your copy over it; **Discard My Changes** downloads the server's file into the copy again (editors that watch their files reload it). Esc leaves the question for later; **Resolve Conflict** on the file's row asks again, and so does the next save.
- **How the file is written**: through a temporary `.part` file next to it that replaces it when complete, so a dropped connection never leaves a half-written file, and with the permissions the file had, so a script keeps its execute bit. Editing a symlink writes the file it points to, so the link stays a link. Because the file is replaced, it gets the login user as owner, and the folder must be writable as well as the file. A file the login user cannot write shows the server's error (such as "Permission denied"); to edit it, use `sudo` with an editor in the terminal.
- **While the tab is disconnected**, saves wait and upload once it reconnects.
- **Editing ends** with **Stop Editing**, closing the tab, or locking Hatoba, and the copy is deleted then. A file whose last save did not upload asks first, "Upload the changes to “name”?": **Upload** or **Discard Changes** (closing the tab can also be cancelled). After the lock, that question comes once the vault is unlocked again. Copies left behind by a crash are deleted at the next start.

Other labels: **Edit**, **Editing**, **Upload**, **Discard Changes**.

Other messages:

- "Its last save hasn’t uploaded to … yet."
- "Its last save hasn’t uploaded to …, and the tab isn’t connected. Reconnect to upload it."
- "Its last save hadn’t uploaded when Hatoba locked."
- "Couldn’t open “name” in an editor. You can choose one in Settings → Terminal." (no editor started: check the program chosen in Settings, or choose one)
- "The file didn’t upload." / "… files didn’t upload." (closing the tab or stopping the edit found an upload that failed; the tab stays open)

## Transfers

Uploads and downloads show in a **Transfers** section at the bottom of the panel, one row each, with a progress bar, percentage, speed and time left. The × cancels a running transfer, or dismisses a finished one (**Cancel transfer** and **Dismiss**). A download or upload can fail midway; the row then shows the error in red. The list refreshes after an upload finishes.

Other labels: **Cancelled**, **Done**.

## Not supported

A built-in editor, editing a file the login user cannot write (through `sudo`), uploading or downloading folders, moving files by drag and drop inside the panel, and changing permissions. For those, use commands in the terminal (`sudo`, `scp`, `rsync`, `chmod`) or an editor on the server.

The error "The SFTP subsystem isn’t available." is explained in `references/troubleshooting-connections.md`, and other file errors in `references/troubleshooting.md`.
