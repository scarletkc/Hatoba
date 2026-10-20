import { applyD1Migrations } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { beforeEach } from "vitest";

await applyD1Migrations(env.DB, env.TEST_MIGRATIONS);

// Storage is shared by every test in a file, so start each test from an empty table.
beforeEach(async () => {
  await env.DB.prepare("DELETE FROM checks").run();
});
