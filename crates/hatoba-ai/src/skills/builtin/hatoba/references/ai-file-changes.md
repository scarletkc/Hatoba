# Reverting the assistant's file changes

Servers rarely keep a history of their configuration files, so Hatoba keeps one for the assistant's **Edit file** and **Write file** calls (`references/ai-panel.md`). Every call that wrote a file can be reverted from its block in the conversation.

| en | zh-CN | ja |
|---|---|---|
| Revert | 撤销修改 | 元に戻す |
| Reverted | 已撤销 | 元に戻し済み |
| Delete | 删除 | 削除 |
| Revert and Send | 撤销并发送 | 元に戻して送信 |
| Keep Files and Send | 保留文件并发送 | ファイルはそのままで送信 |
| Send Anyway | 仍然发送 | このまま送信 |
| Restore Anyway | 仍然恢复 | このまま復元 |
| Delete Anyway | 仍然删除 | このまま削除 |

## Where the earlier content is kept

- Before writing, the call keeps what the file held (or that it did not exist) together with what it writes. This copy stays on this device, encrypted in the vault's local database. Nothing is written on the server, so no backup file can be picked up by a wildcard include such as nginx's `include sites-enabled/*`.
- So a write that failed partway (a timeout, a dropped connection) can be reverted too: its block says the file may be cut short.
- It is not synced and never sent to the model, so only the device that ran the call can revert it.
- The copies hold at most 32 MB of file text in all; the oldest are dropped first, and their blocks then say the change can no longer be reverted. Deleting the conversation deletes them, and so does **Edit and Resend** of a message before the call (below).

## Reverting

Choose **Revert** on the call's block: the icon in its header, shown while the pointer or keyboard focus is on the header, or the button in the expanded block. The expanded block of an Edit file or Write file call shows its change as a diff (**Show input** shows the call's raw arguments). Revert is offered once the assistant has finished (no turn running, no approval waiting, no compaction), and needs the conversation's tab connected to the same server. It asks first, with the server and the diff it will write back, from what the call wrote to what the file held before.

- Revert reads the file first. It writes the earlier content back only while the file still holds exactly what the call wrote. A file that already holds its earlier content (a revert that went through but whose result was lost) is only marked reverted.
- Otherwise it leaves the file as it is and shows the diff from what the file holds now back to what it held before. The file changed since (by you, another program, or a later call of the assistant), or a write stopped partway. When a later call wrote the file too, the block says so, with a link to that call's block: reverting the later calls first, newest first, may bring the file back to where this one can be reverted. **Restore Anyway** under the diff, after a confirmation, writes the earlier content back over exactly what the diff shows, and refuses if the file changed again. A file that was deleted since is created again with the permissions it had, owned by the user the tab is logged in as.
- A file the call created is deleted instead; its confirmation says so (**Delete**, or **Delete Anyway** when it changed since). When a symbolic link was put in its place, Revert says so and leaves the link and the file it points to alone: check them in the terminal.
- Like the file tools, Revert never uses sudo. When the user the tab is logged in as cannot write the file, it says so; restore it in the terminal with sudo instead.
- The block then shows **Reverted**, and your next message tells the assistant, with a divider "Reverted the change to <path>". The assistant has to read the file again before it changes it.

## Editing a message before the changes

**Edit and Resend** of a message deletes the calls after it, with their blocks and their copies. When those calls changed files this device can still revert, sending first lists each file once, with its server, how many of the calls changed it, and the lines a revert adds and removes, and asks:

- **Revert and Send** reverts each change as **Revert** does, newest call first, deleting a file a call created without asking again, then sends. It reverts only the files on the server the tab is connected to, and is unavailable while the tab is disconnected: cancel, reconnect, and send again. A change that cannot be reverted (the file changed since, another server, permission denied) is listed with the reason; **Send Anyway** deletes its copy, so it can no longer be reverted, and **Cancel** keeps it. Reverts already done stay done when you cancel, and your next message tells the assistant.
- **Keep Files and Send** leaves the files as they are.

When the deleted calls ran on another device, or their copy was dropped, the edit box says that their changes stay on the host.

## What Revert does not undo

Revert puts back the file's content only. Commands run since, and a service that already loaded the changed file, stay as they are: after reverting a configuration file, reload the service again if it loaded the change. For the same reason the assistant is told to check a configuration with the service's own test, such as `nginx -t`, `sshd -t` or `visudo -c`, before it reloads or restarts the service.
