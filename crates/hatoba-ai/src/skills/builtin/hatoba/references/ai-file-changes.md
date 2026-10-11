# Reverting the assistant's file changes

Servers rarely keep a history of their configuration files, so Hatoba keeps one for the assistant's **Edit file** and **Write file** calls (`references/ai-panel.md`). Every call that wrote a file can be reverted from its block in the conversation.

| en | zh-CN | ja |
|---|---|---|
| Revert | 撤销修改 | 元に戻す |
| Reverted | 已撤销 | 元に戻し済み |
| Delete | 删除 | 削除 |

## Where the earlier content is kept

- Before writing, the call keeps what the file held (or that it did not exist) together with what it wrote. This copy stays on this device, encrypted in the vault's local database. Nothing is written on the server, so no backup file can be picked up by a wildcard include such as nginx's `include sites-enabled/*`.
- It is not synced and never sent to the model, so only the device that ran the call can revert it.
- The copies hold at most 32 MB of file text in all; the oldest are dropped first. Deleting the conversation deletes them, and so does **Edit and Resend** of a message before the call.

## Reverting

Expand the call's block and choose **Revert**. It is offered once the assistant has finished (no turn running, no approval waiting, no compaction), and needs the conversation's tab connected to the same server.

- Revert reads the file first. It writes the earlier content back only while the file still holds exactly what the call wrote. Otherwise it leaves the file as it is and shows the changes made since the call: by you, by another program, or by a later call of the assistant. Reverting the later calls first, newest first, brings the file back to where this one can be reverted.
- A file the call created is deleted, after a confirmation (**Delete**).
- Like the file tools, Revert never uses sudo. When the user the tab is logged in as cannot write the file, it says so; restore it in the terminal with sudo instead.
- The block then shows **Reverted**, and your next message tells the assistant, with a divider "Reverted the change to <path>". The assistant has to read the file again before it changes it.

## What Revert does not undo

Revert puts back the file's content only. Commands run since, and a service that already loaded the changed file, stay as they are: after reverting a configuration file, reload the service again if it loaded the change. For the same reason the assistant is told to check a configuration with the service's own test, such as `nginx -t`, `sshd -t` or `visudo -c`, before it reloads or restarts the service.
