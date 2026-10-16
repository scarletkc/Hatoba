# Troubleshooting: Cloud Sync and Cloudflare

Local use never depends on sync: hosts, keys and sessions keep working when sync fails. Setup and upgrade steps are in `references/sync.md` and `references/sync-worker.md`. The messages are shown exactly as the app prints them in each language.

## Sync states

The sidebar's sync button and the Cloud Sync page show one of these. Changes made meanwhile stay on the device and upload later.

Other labels: **Will retry automatically**, **Enter your master password again**, **Stored on this device only**.

| State | What it means | What to do |
|---|---|---|
| Offline | The Worker (or Cloudflare) can't be reached | Check the network and the Worker URL (**Endpoint** on the status page). It retries by itself; **Sync Now** retries at once |
| Signed out | The session expired or was revoked (another device pressed **Remove** on this one, or the master password changed) | Enter the master password under **Sign In Again** on the status page |
| Sync error | A sync round failed; the page shows the message | Read the message below; it retries automatically |
| Worker update required | The Worker is too old for this Hatoba | **Update Worker**; see `references/sync-worker.md` |
| Hatoba update required | The Worker is newer than this Hatoba supports | Update Hatoba: Settings → About → **Check for Updates** |
| Sync is off | No sync configured on this device | Set it up under **Cloud Sync** |

## Messages

- "Sync failed."
- "Can’t reach the sync service. Check your network."
- "Cloud Sync signed out. Enter your master password again."
- "The cloud already holds a vault. Go back and connect again to join it."
- "The cloud has no vault yet. Turn on Cloud Sync on a device that has your data first."
- "Another Cloud Sync setup is still running. Try again when it finishes."
- "Cloud Sync is already on for this device. Disconnect it before joining another cloud vault."
- "Connection failed. Check the URL and your network."
- "Enter the full Worker address, e.g. https://hatoba-sync.example.workers.dev."
- "Incorrect master password."
- "Cancelled."

- "The cloud already holds a vault…" appears when the Worker or database got a vault after step 2 checked it (for example another device set it up at the same moment). Go back to step 2 and connect again: it then offers to join that vault (`references/sync.md`, "Join a cloud vault").
- "The cloud has no vault yet…" appears when restoring from a Worker or database that was never set up. Set sync up first on the device that has the data.
- "Another Cloud Sync setup is still running…" appears when a setup or join started earlier has not finished, for example one left running in the background by closing the wizard. Only one runs at a time. Wait for it; the status page then shows the result.
- "Cloud Sync is already on for this device…" appears when joining a cloud vault while this device already syncs, for example because an earlier setup finished in the meantime. To move the device to another cloud vault, use **Disconnect…** on the status page first, then join.
- "Incorrect master password." during setup or restore: enter the master password of the vault that was uploaded, not a new one. If it changed on another device, use the new one. When joining a cloud vault, it is the cloud vault's master password, not this device's.
- For a Worker URL, use the full address including `https://`. A quick check is opening `<Worker URL>/v1/health` in a browser: it should return JSON with `"service": "hatoba-sync"`, a `version`, and `initialized`.

## An item too large to sync

Sync has a size limit per item. With sync off, a host can be saved larger than that (long notes, AI notes and many environment variables), and so can the settings (long custom instructions). Once sync is on, such an item does not upload: it stays on this device and in the pending count, every round tries it again, and everything else syncs as usual. The status page shows a notice for each one:

- A host: "Host “name” is too large to sync. Shorten its notes, AI notes, or environment variables.", with **Edit Host**. Shorten them and **Save**; the next round uploads it.
- The settings: the message below, with **Open Settings → AI**, where **Custom Instructions** is.

While sync is on, saving refuses such a host or instructions up front, with one of these messages:

- "Your settings are too large to sync. Shorten your custom instructions."
- "This host is too large to sync. Shorten its notes or environment variables."

## Cloudflare and deployment errors

These come from **Deploy a Worker from Hatoba** and from direct D1 mode:

- "Cloudflare didn’t accept this API token. Check that it’s complete and not expired or disabled. An account-owned token also needs the account ID."
- "Cloudflare returned an error. Try again later."
- "Choose a workers.dev subdomain for this account."
- "This workers.dev subdomain is taken. Choose another."
- "Another Worker already has this name, and it isn’t a Hatoba sync Worker. Choose another name."
- "This account has no Hatoba Worker with this name, or it holds no vault. Check the account and the Worker name."
- "This Worker is newer than the one in this version of Hatoba, and Hatoba won’t replace it with an older one."
- "This build of Hatoba doesn’t include the sync Worker, so it can’t deploy it. Use another way to deploy."
- "This token can’t edit D1. Add “Account · D1 · Edit” to it in Cloudflare."
- "Can’t reach Cloudflare. Check your network and try again."

- Token rejected: the token is incomplete, expired, disabled, or an account-owned token without the **Account ID**. Create a new one with **Create a token in Cloudflare**; the link fills in both permissions.
- Missing permission: the message names the dashboard permission, "Account · Workers Scripts · Edit" or "Account · D1 · Edit" (direct D1 mode needs only D1). Edit the token in Cloudflare (**Edit token in Cloudflare**), add it, and retry.
- Cloudflare error (code N): Cloudflare returned an error; the code is shown. Wait and **Retry**; check Cloudflare's status page if it persists.
- The workers.dev subdomain: choose one when the account has none; if it is taken, choose another. A new subdomain can take a few minutes to answer; the page waits with **Check again**.
- Name taken: another Worker (not a Hatoba sync Worker) has the name, or the Worker already holds a vault. Hatoba never overwrites either. Choose another Worker name under **Worker and database names**, or restore the existing vault on a new device.
- A Worker is newer: the Worker already runs a newer version than this Hatoba carries; Hatoba does not downgrade it.
- This build has no Worker: a development build without the bundled Worker cannot deploy in-app. Use the Deploy to Cloudflare button or wrangler (**Connect a deployed Worker**).
- A failed step shows the error; **Retry** continues from it, and **Remove what Hatoba created** removes only what this deployment made.

Other labels: **Restore from Cloud**, **Check the token and account**, **Look for an existing Worker and database**, **Create the database**, **Create the tables**, **Upload the Worker**, **Set the setup token**, **Turn on the workers.dev URL**, **Wait for the Worker to answer**, **Account ID (optional)**.

## Devices and conflicts

- A device that should not sync any more: **Remove** it on the status page (**Devices**). It then needs the master password to sync again; no data is deleted.
- "N conflicts were resolved automatically" is not an error. The newer edit wins; for SSH keys the loser is kept as a copy. Open **Review** to keep the result or restore the other version.
- Two devices that show different data: press **Sync Now** on both; a device that is offline or signed out has not uploaded yet.

## Still not solved

If the problem is not covered here and looks like a bug, use the feedback path in SKILL.md ("Project, feedback and newer information"): **Report a Problem** in Settings → About opens GitHub's bug report form (say whether sync uses a Worker or direct D1, and the Worker's version from `/v1/health`). Offer to draft the text, remind the user to leave out Worker URLs they consider private, tokens, keys, passwords and recovery codes, and let the user submit it.
