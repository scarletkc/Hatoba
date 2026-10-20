# Cloud Sync

Cloud Sync keeps the vault the same on several devices. It is optional and off until set up. Hatoba has no sync server: the data goes to a Worker and a D1 database in the user's own Cloudflare account (or straight to D1), always encrypted on the device first, so the cloud holds ciphertext only. Setting up, deploying and upgrading the Worker is in `references/sync-worker.md`.

Open it from the sidebar (**Cloud Sync**, under **Vault**). The sync button at the bottom of the sidebar shows the current state and opens the same page.

Other labels: **Sync is off**, **Stored on this device only**, **Will retry automatically**, **Enter your master password again**, **Resolved automatically, review pending**.

## What syncs

- Synced (encrypted): hosts and saved passwords, groups, SSH keys with their passphrases, trusted host fingerprints, port forwards, terminal settings, AI providers with their API keys and models, the search provider, the default model, the judge of Auto mode, the custom instructions, the Terminal Screen default, AI conversations, the assistant's memories and the memory switch, skills (and the switch of the built-in skill), and MCP server definitions (with the **Always ask** setting).
- Not synced, kept on each device: language, theme, list density, host reachability, the Auto-Lock and other lock settings, the clipboard delay, the update-check switch, the AI permission mode, the saved AI approvals and the AI panel size, whether an MCP server is switched on for the device, and the MCP "Always allow" switches.
- A sync round runs after unlock, about 2 seconds after a local change, every 60 seconds, when the window regains focus, and on **Sync Now**. Local work never waits for it.

## Set up on the first device

With sync off, the page is a three-step wizard (**Method**, **Connect**, **Password**). A vault must already exist locally.

Other labels: **Recommended**, **Continue**, **Cancel**, **Back**.

1. **Method**: **Deploy a Worker from Hatoba** (recommended; Hatoba creates the Worker and database with a Cloudflare API token), **Connect a deployed Worker** (the user deployed it with the Deploy to Cloudflare button or wrangler and enters its URL and Setup Token), or **Use an API Token** (direct D1 mode: no Worker, the app calls the D1 API; the token can usually edit every D1 database in the account). **How is my data encrypted?** explains the encryption.
2. **Connect**: the details of the chosen method; see `references/sync-worker.md`.
3. **Password**: **Enter Your Master Password** (the existing one, not a new one), then **Finish Setup**. Hatoba uploads the vault ("Uploading N items…") and the page switches to the status page.

Other labels: **Cloud Sync is on**.

## Join a cloud vault

When the Worker or database chosen in step 2 already holds a vault, step 3 joins it instead of setting it up. This is how a device that already has its own vault (or synced before and was disconnected) gets the cloud vault: it does not need a fresh install.

- Step 3 is **Enter the Cloud Vault’s Master Password**: the master password of the vault in the cloud, not this device's. **Merge and Turn On Sync** signs in and merges.
- This device's hosts, keys and other items are added to the cloud vault, and the cloud's items arrive on this device. The cloud's settings are kept. If this device synced with that same vault before, matching items are reconciled instead of duplicated, and the newer edit wins.
- From then on this device unlocks with the cloud vault's master password, and the cloud vault's recovery code replaces this device's, which stops working. When the cloud held another vault, **Unlock with Windows Hello** (or **Unlock with Touch ID**) and **Unlock with PIN** are turned off; turn them on again in Settings → Security.

Other labels: **Connected · the cloud already holds a vault**, **Merging…**.

## Add another device

On a new device's first launch choose **Restore from Cloud**: pick the method (**Sync Worker** or **Direct D1**), enter the Worker URL (or the account ID, token and database), use **Test Connection**, then enter the master password and **Restore Vault**. Data is downloaded and decrypted on the device. A device that already has its own vault joins from the Cloud Sync page instead (see "Join a cloud vault").

## The status page

The header shows the state ("Synced", "Syncing…", "Offline", "Signed out", "Sync error", or a paused state) with the last sync time and counts of hosts, keys and groups, and the **Sync Now** button.

- **Connection** lists the **Method**, **Endpoint** (the Worker URL) and **Database**, and the **Sync automatically** switch. With it off, sync runs only on **Sync Now**.
- **Offline**: the network or the service is unreachable. Local changes are kept and uploaded automatically later. See `references/troubleshooting-sync.md`.
- **Signed out**: the session expired or was revoked. A master password field with **Sign In Again** appears on the page.
- A paused state (**Worker update required** or **Hatoba update required**) means the Worker and the app versions no longer match; see `references/sync-worker.md`.
- A notice that a host or the settings are too large to sync means that item can't upload until it is shortened; see `references/troubleshooting-sync.md`.
- **Devices** lists the signed-in devices (**This PC** marks this one) with the platform and the last seen time. A device not seen for 14 days or more is flagged "Not seen in 14+ days". **Remove** next to another device (then **Remove Device**) revokes its sign-in. It needs the master password again to sync, and no data is deleted.

## Conflicts

When two devices edit the same item, Hatoba resolves it automatically: the newer edit wins; an edit beats a delete; for SSH keys the losing version is kept as a second key whose name carries a conflict-copy suffix. Sync does not stop. A banner on the status page says how many conflicts were resolved, with **Review**.

Other messages:

- "Kept both versions; one is marked “(conflict copy)”"
- "No conflicts to review."

The review page lists each item with **This PC** and **Cloud** versions. **Keep Current Result** accepts what Hatoba chose; **Restore Other Version** switches to the other side.

## Security actions and disconnecting

At the bottom of the status page:

- **Generate New Recovery Code** and **Change Master Password** work like their Settings → Security counterparts. After a password change, the other devices are signed out and must sign in with the new one.
- **Disconnect…** stops syncing on this device, after a confirmation ("Disconnect Cloud Sync?"). Data stays on the device and nothing is deleted from the cloud. You can reconnect through the wizard later.
