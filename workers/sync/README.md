# Deploy the Hatoba sync Worker

Hatoba's sync backend is a Worker (Hono) and a D1 database running in **your own Cloudflare account**, with no dependency on any official Hatoba server. One deployment serves one user (a single vault). The server stores only ciphertext and never sees the master password, `vault_key`, or any plaintext.

This page covers deployment, upgrades, and maintenance. The API and server behavior are in [§6.2 of the architecture and requirements](../../docs/hatoba-spec.md#62-worker-api), the security properties in [§4.4 Threat model](../../docs/hatoba-spec.md#44-threat-model) and [§4.5 Data visible to the server](../../docs/hatoba-spec.md#45-data-visible-to-the-server), and local development and testing in the [development guide](../../docs/development.md#sync-worker).

## Before you start

You need:

- A Cloudflare account. The free plan works, and personal use usually stays within its limits.
- Node.js **22 or later** (required by wrangler 4) and npm.

Run every command below in this directory (`workers/sync`). They work the same in Windows PowerShell and on macOS and Linux.

## Deploy

### 1. Install dependencies and sign in

```sh
npm install
npx wrangler login
```

### 2. Create the D1 database

```sh
npx wrangler d1 create hatoba
```

Add `--location apac|weur|eeur|oc|wnam|enam` to pick a region near you, for example `npx wrangler d1 create hatoba --location apac`.

The command prints a `database_id`. Put it in `[[d1_databases]]` in [`wrangler.toml`](./wrangler.toml), replacing the placeholder `00000000-0000-0000-0000-000000000000`:

```toml
[[d1_databases]]
binding = "DB"                  # Do not change it, the code depends on this name
database_name = "hatoba"
database_id = "<the id that wrangler printed>"
migrations_dir = "migrations"
```

If wrangler offers to add the binding to the configuration for you, answer no and fill it in by hand. If you answered yes, make sure the configuration has no duplicate `[[d1_databases]]` section.

### 3. Create the tables (migrations)

```sh
npx wrangler d1 migrations apply hatoba --remote
```

Without `--remote`, the migrations apply only to the local development database.

### 4. Set the setup token

The setup token stops someone else from initializing a freshly deployed, unconfigured Worker before you do. It is a random string that only you know, so **use a strong random value**:

```sh
# macOS / Linux / Git Bash
openssl rand -base64 32

# Any system with Node installed
node -e "console.log(require('crypto').randomBytes(32).toString('base64url'))"

# Windows PowerShell (no openssl needed)
$b = New-Object byte[] 32; [Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($b); [Convert]::ToBase64String($b)
```

Save the value in your password manager (you paste it into Hatoba later), then store it as a Worker secret:

```sh
npx wrangler secret put SETUP_TOKEN
```

Paste the value when prompted. If wrangler says there is no Worker named hatoba-sync yet and asks whether to create one, answer yes. The secret never appears in `wrangler.toml` or in git.

### 5. Deploy

```sh
npx wrangler deploy
```

The output includes the Worker's URL, such as `https://hatoba-sync.<your-subdomain>.workers.dev`. Check that the deployment works:

```sh
curl https://hatoba-sync.<your-subdomain>.workers.dev/v1/health
```

A JSON response with `service` set to `"hatoba-sync"` and `initialized` set to `false` means the Worker is deployed and not initialized yet. A `503 database_unavailable` means the `database_id` from step 2 is wrong or step 3 did not run.

### 6. Enable sync in Hatoba

1. In Hatoba, open **Cloud Sync** in the sidebar and choose **Deploy a Worker**.
2. Enter the **Worker URL** from step 5 and the **Setup Token** from step 4, and select **Test Connection**.
3. Follow the wizard to set or enter the master password. The first device calls `/v1/setup` to initialize the vault, then signs in and pushes every item.

To add another device, choose **Restore from Cloud** on its first launch. It needs only the Worker URL and the master password, **not** the setup token.

## Maintenance

### After initialization (optional hardening)

Once the vault is initialized, the setup token is no longer needed. You can delete it, and `/v1/setup` then always returns `503 setup_token_not_configured`:

```sh
npx wrangler secret delete SETUP_TOKEN
```

### Upgrade

```sh
git pull
npm install
npx wrangler d1 migrations apply hatoba --remote   # Applies only the new migrations
npx wrangler deploy
```

### Reset (discard the cloud vault)

Use this only when you are sure you want to wipe the cloud data, for example after entering something wrong during setup. Data on your devices is not affected:

```sh
npx wrangler d1 execute hatoba --remote --command "DELETE FROM sessions; DELETE FROM items; DELETE FROM meta;"
```

### Back up and inspect

```sh
npx wrangler d1 export hatoba --remote --output hatoba-backup.sql
```

The export holds only ciphertext, KDF parameters, and hashes, and a search finds no host name, password, or private key in it. This is also how you can check that the server holds only ciphertext.
