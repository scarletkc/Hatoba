# Troubleshooting: keys, vault, files, updates and other messages

The messages are shown exactly as the app prints them in each language, so a message the user pastes can be matched to a row. Failed SSH connections, jump hosts, host key prompts and dropped connections are in `references/troubleshooting-connections.md`. Sync and Cloudflare errors are in `references/troubleshooting-sync.md`, AI errors in `references/troubleshooting-ai.md`.

## Key errors

When importing a key, or connecting with one:

- "Unsupported private key format. Use OpenSSH, PEM or PuTTY (.ppk)."
- "This key is protected by a passphrase. Enter it to continue."
- "Incorrect passphrase for this key."
- "Unsupported key algorithm. Ed25519, ECDSA and RSA are supported."
- "The private key is invalid or damaged."
- "This private key can’t be read."

- Supported formats are OpenSSH, PEM and PuTTY (.ppk); supported algorithms are Ed25519, ECDSA and RSA (for example, DSA keys are not).
- Enter the passphrase in the import form (it is saved encrypted with the key) or when Hatoba asks for it at connect; a wrong one asks again. A key on a jump host needs its passphrase saved.
- A key that is "invalid or damaged" is usually truncated: copy the whole text including the BEGIN and END lines, or import the file instead of pasting.
- Deleting a key switches the hosts that used it to **Ask Each Time**.

## Locked, password and recovery

- "The vault is locked."
- "Incorrect master password."
- "That recovery code isn’t right. Check it and try again."
- "The two passwords don’t match."
- "The master password needs at least 8 characters."
- "This password is easy to guess. A longer or less common one is safer."
- "No vault has been created yet."
- "A vault already exists."

- After wrong master passwords, "Too many attempts. Try again in N s." appears and the password field is disabled until the countdown ends. Wait; each further miss lengthens the delay, up to a minute.
- A forgotten master password can be reset only with the recovery code (**Forgot it? Use your recovery code**). With neither, the data cannot be recovered, and Hatoba cannot reset it.
- The recovery code ignores case, dashes and spaces. A new code can be generated in Settings → Security (**Generate New Code…**), after which the old one stops working.

## Files, forwards and other messages

- "The file operation failed."
- "Couldn’t read or write the file."
- "Something went wrong."
- "This item no longer exists. It may have been deleted on another device."
- "Cancelled."
- "Can’t access the clipboard."
- "The name can’t be empty or contain /."

Other labels: **Can’t read this folder**.

- "The file operation failed." (SFTP panel) means the server refused the operation: permissions, a name that already exists, or a full disk. See `references/sftp.md`. Folders cannot be uploaded or downloaded.
- "Couldn’t read or write the file." is a local problem, such as a path that cannot be written when saving a download, a backup, or a skill export.
- "This item no longer exists." usually means it was deleted on another device; refresh the list or recreate it.
- "Something went wrong." is a generic failure. Retry once, then use the feedback path below.
- Port forwarding failures show in the **Port Forwarding** popover; a local port that is already in use is the usual cause (edit the rule, or use local port 0). See `references/port-forwarding.md`.
- On Windows the logs are in `%LOCALAPPDATA%\app.hatoba.desktop\logs`. They record each SSH connection attempt with its host, port, user name, authentication method and kind of error, and never passwords, keys or terminal content.

## Updates

These show in Settings → About, in the update row, after **Check for Updates** or **Download and Install**:

- "Can’t reach GitHub. Check your network and try again."
- "Couldn’t check for updates. Try again later."
- "Couldn’t install the update. Try again later."
- "The downloaded update isn’t signed by Hatoba, so it wasn’t installed."

- "Can’t reach GitHub." means GitHub did not answer at all: no network, a DNS or firewall problem, or a download that received nothing for 30 seconds. Check the connection and press the button again; a download starts over.
- "Couldn’t check for updates." means GitHub answered with an error. Try again later.
- "Couldn’t install the update." means the download or the installer failed for another reason, and the current version keeps running. Press **Download and Install** again. If it keeps failing, open the release page with **View on GitHub** under the release notes, download the installer for the platform and run it, and report the problem with the logs.
- The signature message means the downloaded file did not match the signature of Hatoba's release key, so nothing was installed. Do not install that version by hand from its release page, which may hold the same rejected file. Report it with **Report a Problem**, keep the current version, and wait for a correctly signed update.

## Still not solved

If a problem is not covered by the troubleshooting files and looks like a bug, or the user wants something Hatoba lacks, use the feedback path in SKILL.md ("Project, feedback and newer information"): **Report a Problem** in Settings → About opens GitHub's bug report form, and feature requests go to https://github.com/scarletkc/Hatoba/issues/new?template=feature_request.yml. Offer to draft the text, and remind the user to leave out host names, addresses, keys, passwords and other secrets. The user submits it.
