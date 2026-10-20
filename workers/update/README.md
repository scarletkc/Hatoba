# Update counter

The Worker behind `hatoba.fog.moe/update/*`. An installed stable version of Hatoba asks it for its update manifest at `/update/<os>/<arch>/<package type>/<version>`, the first of the updater's `endpoints` in [`tauri.conf.json`](../../apps/desktop/src-tauri/tauri.conf.json). The Worker adds the check to that day's count for the version and platform, then redirects to the `latest.json` of the latest stable release on GitHub. What it keeps, and what it does not, is in the Privacy row of [§11 of the architecture and requirements](../../docs/hatoba-spec.md#11-non-functional-requirements).

It runs in the project's Cloudflare account, on the `fog.moe` zone, unlike the [sync Worker](../sync/README.md), which each user deploys to their own account.

## Deploy

With wrangler logged in to that account (`npx wrangler login`), in this directory:

```sh
npm install
npm run deploy    # applies the migrations, then deploys the Worker and its route
```

The D1 database `hatoba-update` was made once with `npx wrangler d1 create hatoba-update`; its ID is `database_id` in `wrangler.toml`.

## Read the counts

```sh
npm run stats
```

It prints the checks of each of the last 30 days (UTC), and of the last 7 days by version and by platform. A check is one update check of one installation: an installation that keeps running counts about once a day, and one that is launched several times a day counts once per launch. Installations with the automatic check turned off count only when the user checks.

Other questions go to the `checks` table ([`migrations/0001_init.sql`](migrations/0001_init.sql)), for example:

```sh
npx wrangler d1 execute DB --remote --command "SELECT version, SUM(count) FROM checks GROUP BY version"
```

## Test

```sh
npm run typecheck
npm test          # Vitest on local workerd with a local D1
```
