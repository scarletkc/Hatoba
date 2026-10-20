# Keys, vault and lock

## The vault and the master password

Hosts, SSH keys, saved passwords, known host fingerprints, forwards, AI settings, skills and settings live in an encrypted vault on the device. Each item is encrypted with AES-256-GCM using a random vault key, and the vault key is encrypted with a key derived from the master password (Argon2id). The master password never leaves the device. Hatoba does not store it and cannot reset it.

On first launch the window offers two ways to start:

- **Create a New Vault** sets a master password (at least 8 characters, with a strength hint: Weak, Fair, Strong, Very strong; a password that is easy to guess gets a warning but can be used) and then shows a recovery code.
- **Restore from Cloud** is for a new device that should join an existing sync setup. A device that already has a vault joins from the Cloud Sync page. See `references/sync.md`.
- The recovery code is shown once. **Save as Text** or **Print** it, then tick the box to confirm it is saved; **Finish Setup** completes the setup. It is the only way back in if the master password is forgotten. A new code can be generated in Settings → Security (**Generate New Code…**), and then the old one stops working.

Other labels: **Start**, **Connect**, **Save Your Recovery Code**.

## Locking and unlocking

Lock with the lock button at the bottom of the sidebar, Ctrl+Shift+L (⌘L on macOS), or Settings → Security → **Lock Now**. Hatoba also locks when the computer has had no keyboard or mouse input for the time set in **Auto-Lock** (on Linux, only input in Hatoba's window counts), and when the computer goes to sleep, unless **Lock when the computer sleeps** is off.

- Auto-Lock offers 1, 5, 15 (the default), 30 minutes, 1, 4 or 8 hours, or Never. Auto-Lock, **Lock when the computer sleeps** (on by default), **Disconnect all sessions when locked** and **Clear Clipboard** are kept on each device and do not sync.
- Open SSH sessions stay connected behind the lock screen ("N sessions stay connected in the background"), unless **Disconnect all sessions when locked** is on. The AI assistant stops working and MCP servers stop when the vault locks.
- The lock screen asks for the master password (**Unlock**). The eye button in the field shows what was typed, until the window loses focus. After wrong attempts the field is disabled for a growing delay, with a countdown such as "Too many attempts. Try again in 30 s."
- **Forgot it? Use your recovery code** opens **Reset with Recovery Code**: enter the recovery code (case and dashes do not matter) and a new master password, then **Reset and Unlock**. It works offline and loses no data.
- **Unlock with Windows Hello** (Windows) or **Unlock with Touch ID** (macOS) is a switch in Settings → Security. It appears only on a Windows PC where Windows Hello is set up or on a Mac with Touch ID, and asks for the master password once to turn on. The lock screen then shows a button for it, and asks on its own once per lock when the user comes back: at once when Hatoba starts in front, when its window gets the focus, or after an idle or sleep lock at the first mouse movement in the window. After a manual lock it waits until the window has been left and is focused again. Cancelling leaves the button and the password field. Touch ID can't be used while the Mac's lid is closed; the lock screen says so when the button is clicked. If the saved unlock stops working on the device (on a Mac, for example after fingerprints are added or removed), the lock screen says so and turns the switch off, and it can be turned on again in Settings → Security.
- **Unlock with PIN** is a switch in Settings → Security, on every platform. Turning it on asks for the master password and a PIN of at least 4 characters of any kind (**Set Up PIN**). After the first unlock of each launch, the lock screen asks for the PIN (**Enter your PIN to unlock the vault**), with **Use master password** to switch, and **Use PIN** to switch back. A wrong PIN says how many tries are left; after five wrong PINs in a row the lock screen asks for the master password, and the PIN works again after an unlock with it. To change the PIN, turn the switch off and on again.
- **Require the master password after a restart** appears under the PIN switch and is on by default. Turned off, the PIN also unlocks after Hatoba or the computer restarts: the vault key, wrapped by the PIN, is kept in the system credential store, so anyone who can read that store as the user can try PINs without the limit of five. The PIN and this switch are kept on each device and do not sync; joining a cloud vault that is a different vault turns the PIN off.

Other labels: **1 hour**, **Hatoba is locked**, **New Master Password**, **Confirm PIN**, **Turn On**.

## Master password, backup and clipboard

- Settings → Security → **Master Password** → **Change…** re-encrypts only the vault key; saved hosts and keys are untouched. With Cloud Sync on, the other devices are signed out and need the new password. The Cloud Sync page has the same action.
- Settings → General → **Export Encrypted Backup** → **Export…** saves the vault as one `hatoba-backup-YYYY-MM-DD.hatoba` file that only the master password opens. The app has no screen to import a backup file.
- **Copy Password** on a host clears the copied password from the clipboard after the time set in Settings → Security → **Clear Clipboard**: 90 seconds by default, or Never.

Other labels: **Change Master Password**.

## The Keys page

Open it from the sidebar (**Keys**, under **Vault**). It lists the SSH keys in the vault, searchable (**Search keys**), with the buttons **Import…** and **Generate Key**. Columns are Name, Type, Fingerprint, Created and Used By (the first host using the key, plus how many more, or **Unused**). Selecting a key opens a detail panel with its fingerprint, creation date, last use, the public key (**Copy**), the hosts that use it, and the actions **Rename**, **Deploy to Host…** and **Delete…**.

Other labels: **Public Key**, **Used by**, **Last used**, **Copy public key**.

- **Import…** opens **Import Private Key**. Choose a file (**Choose File**) or **Paste Text**, give the key a name, and enter its passphrase if it has one (the passphrase is saved encrypted with the key). OpenSSH, PEM and PuTTY (.ppk) formats are supported; the algorithms are Ed25519, ECDSA and RSA.
- **Generate Key** makes an Ed25519 key (recommended) or an RSA 4096 key on this device, with a comment and an optional passphrase. RSA takes a few seconds.
- **Deploy to Host…** picks a host, connects with that host's existing authentication, and appends the key's public key to `~/.ssh/authorized_keys` on the server. After that the host can use **Key** authentication.
- **Delete…** removes the key from the vault for good, on all synced devices. Hosts that used it fall back to **Ask Each Time**.
- The private key cannot be viewed or exported; only the public key can be copied. It is decrypted in memory only while connecting.

Other labels: **Choose File…**, **Passphrase (optional)**, **Comment**, **Confirm Passphrase**, **Rename Key**, **Deploy Public Key to Host**, **Delete Key**.

For key errors (unsupported format, wrong passphrase) see `references/troubleshooting.md`.
