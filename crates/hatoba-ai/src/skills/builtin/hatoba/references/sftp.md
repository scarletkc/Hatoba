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

- **Download…** is for files: it asks where to save the file locally. Folders cannot be downloaded.
- **Upload Files…** (or the upload button) picks local files, and dropping files from the desktop onto the panel uploads them to the folder that is open ("Drop to upload to …"). Only files are uploaded, not folders. An upload to an existing name replaces the file.
- **New Folder** adds a row where you type the name. **Rename** edits the name in place; Enter or leaving the field commits and Esc cancels. A name cannot be empty or contain `/`.
- **Delete…** asks first. A file is deleted for good; a folder is deleted with everything in it ("permanently deleted from the server. This can’t be undone"). The root directory cannot be deleted.
- **Copy Path** copies the full remote path to the clipboard.

Other labels: **Path copied**.

Other messages:

- "The name can’t be empty or contain /."
- "This file will be permanently deleted from the server. This can’t be undone."
- "This folder and everything in it will be permanently deleted from the server. This can’t be undone."

## Transfers

Uploads and downloads show in a **Transfers** section at the bottom of the panel, one row each, with a progress bar, percentage, speed and time left. The × cancels a running transfer, or dismisses a finished one (**Cancel transfer** and **Dismiss**). A download or upload can fail midway; the row then shows the error in red. The list refreshes after an upload finishes.

Other labels: **Cancelled**, **Done**.

## Not supported

Editing a remote file in the app, uploading or downloading folders, moving files by drag and drop inside the panel, and changing permissions. For those, use commands in the terminal (`scp`, `rsync`, `chmod`) or an editor on the server.

The error "The SFTP subsystem isn’t available." is explained in `references/troubleshooting-connections.md`, and other file errors in `references/troubleshooting.md`.
