import { createExecutionContext, waitOnExecutionContext } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import worker, { type Env, MANIFEST } from "../src/index";

const IncomingRequest = Request<unknown, IncomingRequestCfProperties>;

/** Asks the Worker for `path` and waits for the count it starts after answering. */
async function ask(path: string, method = "GET", bindings: Env = env): Promise<Response> {
  const ctx = createExecutionContext();
  const response = await worker.fetch(new IncomingRequest(`https://hatoba.fog.moe${path}`, { method }), bindings, ctx);
  await waitOnExecutionContext(ctx);
  return response;
}

async function counts(): Promise<unknown[]> {
  const { results } = await env.DB.prepare(
    "SELECT day, version, os, arch, bundle, count FROM checks ORDER BY version, os, bundle",
  ).all();
  return results;
}

const today = () => new Date().toISOString().slice(0, 10);

function expectRedirect(response: Response) {
  expect(response.status).toBe(302);
  expect(response.headers.get("Location")).toBe(MANIFEST);
  expect(response.headers.get("Cache-Control")).toBe("no-store");
}

describe("update check", () => {
  it("counts the check and sends the updater to the latest release's manifest", async () => {
    expectRedirect(await ask("/update/windows/x86_64/nsis/0.4.2"));
    expect(await counts()).toEqual([
      { day: today(), version: "0.4.2", os: "windows", arch: "x86_64", bundle: "nsis", count: 1 },
    ]);
  });

  it("adds up checks of the same day, version and platform", async () => {
    for (const path of [
      "/update/windows/x86_64/nsis/0.4.2",
      "/update/windows/x86_64/nsis/0.4.2",
      "/update/linux/x86_64/deb/0.4.2",
      "/update/linux/x86_64/appimage/0.4.2",
      "/update/darwin/aarch64/app/0.4.1",
    ]) {
      expectRedirect(await ask(path));
    }
    const day = today();
    expect(await counts()).toEqual([
      { day, version: "0.4.1", os: "darwin", arch: "aarch64", bundle: "app", count: 1 },
      { day, version: "0.4.2", os: "linux", arch: "x86_64", bundle: "appimage", count: 1 },
      { day, version: "0.4.2", os: "linux", arch: "x86_64", bundle: "deb", count: 1 },
      { day, version: "0.4.2", os: "windows", arch: "x86_64", bundle: "nsis", count: 2 },
    ]);
  });

  it("redirects without counting what an installed stable version does not ask", async () => {
    for (const path of [
      // A development build has no bundle type.
      "/update/windows/x86_64/unknown/0.4.2",
      "/update/windows/x86_64/nsis/0.5.0-rc.1",
      "/update/windows/x86_64/nsis/latest",
      "/update/plan9/x86_64/nsis/0.4.2",
      "/update/windows/x86_64/nsis/0.4.2/latest.json",
      "/update/windows/x86_64/nsis",
      "/update/%7B%7Btarget%7D%7D/%7B%7Barch%7D%7D/%7B%7Bbundle_type%7D%7D/%7B%7Bcurrent_version%7D%7D",
      "/update/",
    ]) {
      expectRedirect(await ask(path));
    }
    expectRedirect(await ask("/update/windows/x86_64/nsis/0.4.2", "POST"));
    expect(await counts()).toEqual([]);
  });

  it("still redirects when the count fails", async () => {
    const broken = {
      prepare() {
        throw new Error("no such table: checks");
      },
    } as unknown as D1Database;
    expectRedirect(await ask("/update/windows/x86_64/nsis/0.4.2", "GET", { DB: broken }));
  });
});
