import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { MAX_ENVELOPE_BYTES } from "../src/config";
import { call, change, envelope, loggedIn, push } from "./helpers";

const ID_1 = "0192a000-0000-7000-8000-000000000001";
const ID_2 = "0192a000-0000-7000-8000-000000000002";
const ID_3 = "0192a000-0000-7000-8000-000000000003";

const uuid = (n: number) => `0192a000-0000-7000-8000-${String(n).padStart(12, "0")}`;

describe("POST /v1/items", () => {
  it("creates a new item at revision 1 with a fresh seq", async () => {
    const token = await loggedIn();
    const res = await push(token, [change(ID_1, 0, "first")]);
    expect(res.status).toBe(200);
    expect(res.body.results).toEqual([{ id: ID_1, status: "ok", revision: 1, seq: 1 }]);

    const row = await env.DB.prepare("SELECT * FROM items WHERE id = ?").bind(ID_1).first();
    expect(row).toMatchObject({ envelope: envelope("first"), revision: 1, seq: 1, deleted: 0 });
    const meta = await env.DB.prepare("SELECT seq FROM meta").first<{ seq: number }>();
    expect(meta!.seq).toBe(1);
  });

  it("applies several changes in order with increasing seq values", async () => {
    const token = await loggedIn();
    const res = await push(token, [change(ID_1, 0), change(ID_2, 0), change(ID_3, 0)]);
    expect(res.body.results.map((r: { seq: number }) => r.seq)).toEqual([1, 2, 3]);
    expect(res.body.results.map((r: { id: string }) => r.id)).toEqual([ID_1, ID_2, ID_3]);
  });

  it("updates an item when base_revision matches and bumps revision and seq", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0, "v1")]);
    const res = await push(token, [change(ID_1, 1, "v2", { updated_at: 1234 })]);
    expect(res.body.results).toEqual([{ id: ID_1, status: "ok", revision: 2, seq: 2 }]);
    const row = await env.DB.prepare("SELECT * FROM items").first();
    expect(row).toMatchObject({ envelope: envelope("v2"), revision: 2, seq: 2, updated_at: 1234 });
  });

  it("uses the server clock when updated_at is omitted", async () => {
    const token = await loggedIn();
    const before = Date.now();
    await push(token, [change(ID_1, 0)]);
    const row = await env.DB.prepare("SELECT updated_at FROM items").first<{ updated_at: number }>();
    expect(row!.updated_at).toBeGreaterThanOrEqual(before);
    expect(row!.updated_at).toBeLessThanOrEqual(Date.now());
  });

  it("returns the server row on a stale base_revision and leaves it untouched", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0, "v1")]);
    await push(token, [change(ID_1, 1, "v2")]);
    const res = await push(token, [change(ID_1, 1, "stale-write")]);

    const [result] = res.body.results;
    expect(result.status).toBe("conflict");
    expect(result.id).toBe(ID_1);
    expect(result.server).toMatchObject({
      revision: 2,
      deleted: false,
      envelope: envelope("v2"),
    });
    expect(result.server.seq).toBe(2);
    expect(typeof result.server.updated_at).toBe("number");
    const row = await env.DB.prepare("SELECT envelope, revision FROM items").first();
    expect(row).toMatchObject({ envelope: envelope("v2"), revision: 2 });
  });

  it("treats base_revision 0 for an existing item as a conflict", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0, "original")]);
    const res = await push(token, [change(ID_1, 0, "duplicate-create")]);
    expect(res.body.results[0]).toMatchObject({
      status: "conflict",
      server: { revision: 1, envelope: envelope("original"), deleted: false },
    });
  });

  it("lets exactly one of two concurrent pushes with the same base_revision win", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0, "v1")]);

    const [a, b] = await Promise.all([
      push(token, [change(ID_1, 1, "from-a")]),
      push(token, [change(ID_1, 1, "from-b")]),
    ]);
    const statuses = [a.body.results[0].status, b.body.results[0].status].sort();
    expect(statuses).toEqual(["conflict", "ok"]);

    const winner = a.body.results[0].status === "ok" ? a : b;
    const loser = winner === a ? b : a;
    const winnerTag = winner === a ? "from-a" : "from-b";
    expect(winner.body.results[0].revision).toBe(2);
    expect(loser.body.results[0].server).toMatchObject({ revision: 2, envelope: envelope(winnerTag) });
    const row = await env.DB.prepare("SELECT revision, envelope FROM items").first();
    expect(row).toMatchObject({ revision: 2, envelope: envelope(winnerTag) });
  });

  it("lets exactly one of many concurrent creates of the same id win", async () => {
    const token = await loggedIn();
    const responses = await Promise.all(
      Array.from({ length: 6 }, (_, i) => push(token, [change(ID_1, 0, `racer-${i}`)])),
    );
    const statuses = responses.map((res) => res.body.results[0].status);
    expect(statuses.filter((s) => s === "ok")).toHaveLength(1);
    expect(statuses.filter((s) => s === "conflict")).toHaveLength(5);
  });

  it("keeps seq strictly increasing across interleaved concurrent pushes", async () => {
    const token = await loggedIn();
    const responses = await Promise.all(
      Array.from({ length: 5 }, (_, i) => push(token, [change(uuid(i + 1), 0), change(uuid(i + 11), 0)])),
    );
    const seqs = responses.flatMap((res) => res.body.results.map((r: { seq: number }) => r.seq));
    expect(new Set(seqs).size).toBe(10);
    // within one request, results are in increasing seq order
    for (const res of responses) {
      const [first, second] = res.body.results;
      expect(second.seq).toBeGreaterThan(first.seq);
    }
  });

  it("allows seq gaps: a lost conflict still consumes a seq number", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0)]); // seq 1
    await push(token, [change(ID_1, 0)]); // conflict, burns seq 2
    const res = await push(token, [change(ID_2, 0)]);
    expect(res.body.results[0].seq).toBe(3);
  });

  it("allocates past items that D1 direct mode wrote before catching meta.seq up", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0)]); // seq 1
    // D1 direct mode writes at MAX(seq) + 1 and raises meta.seq only after the push.
    await env.DB.prepare(
      "INSERT INTO items (id, envelope, revision, seq, deleted, updated_at) VALUES (?, ?, 1, 2, 0, 0)",
    )
      .bind(ID_2, envelope("direct"))
      .run();
    const pulled = await call("/v1/items?since=0", { token });
    const cursor = pulled.body.next_since;
    expect(cursor).toBe(2);

    const res = await push(token, [change(ID_3, 0)]);
    expect(res.body.results[0].seq).toBe(3);
    const after = await call(`/v1/items?since=${cursor}`, { token });
    expect(after.body.items.map((i: { id: string }) => i.id)).toEqual([ID_3]);
  });

  it("stores deletions as tombstones with a NULL envelope", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0, "alive")]);
    const res = await push(token, [{ id: ID_1, base_revision: 1, deleted: true, envelope: null }]);
    expect(res.body.results).toEqual([{ id: ID_1, status: "ok", revision: 2, seq: 2 }]);

    const row = await env.DB.prepare("SELECT envelope, deleted, revision FROM items").first();
    expect(row).toEqual({ envelope: null, deleted: 1, revision: 2 });

    const pulled = await call("/v1/items", { token });
    expect(pulled.body.items[0]).toMatchObject({ id: ID_1, envelope: null, deleted: true, revision: 2 });
  });

  it("accepts a deletion without an envelope field and can create a tombstone directly", async () => {
    const token = await loggedIn();
    const res = await push(token, [{ id: ID_1, base_revision: 0, deleted: true }]);
    expect(res.body.results[0]).toMatchObject({ status: "ok", revision: 1 });
  });

  it("restores a tombstoned item when the next write carries the current revision", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0)]);
    await push(token, [{ id: ID_1, base_revision: 1, deleted: true, envelope: null }]);
    const res = await push(token, [change(ID_1, 2, "restored")]);
    expect(res.body.results[0]).toMatchObject({ status: "ok", revision: 3 });
    const row = await env.DB.prepare("SELECT envelope, deleted FROM items").first();
    expect(row).toEqual({ envelope: envelope("restored"), deleted: 0 });
  });

  it("reports a conflict against a tombstone with deleted=true and no envelope", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_1, 0)]);
    await push(token, [{ id: ID_1, base_revision: 1, deleted: true, envelope: null }]);
    const res = await push(token, [change(ID_1, 1, "edit-after-delete")]);
    expect(res.body.results[0]).toMatchObject({
      status: "conflict",
      server: { revision: 2, deleted: true, envelope: null },
    });
  });

  it("reports not_found when the client expects an item the server has never seen", async () => {
    const token = await loggedIn();
    const res = await push(token, [change(ID_1, 5)]);
    expect(res.body.results).toEqual([{ id: ID_1, status: "error", error: "not_found" }]);
  });

  it("returns a result per change: ok, conflict and ok mixed", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_2, 0, "existing")]);
    const res = await push(token, [change(ID_1, 0), change(ID_2, 0), change(ID_3, 0)]);
    expect(res.body.results.map((r: { status: string }) => r.status)).toEqual(["ok", "conflict", "ok"]);
  });

  it("accepts the reserved 'settings' item id", async () => {
    const token = await loggedIn();
    const res = await push(token, [change("settings", 0)]);
    expect(res.body.results[0]).toMatchObject({ id: "settings", status: "ok" });
  });

  it("accepts an empty change list", async () => {
    const token = await loggedIn();
    const res = await push(token, []);
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ results: [] });
  });

  describe("limits", () => {
    it("accepts 100 changes in one request", async () => {
      const token = await loggedIn();
      const res = await push(token, Array.from({ length: 100 }, (_, i) => change(uuid(i + 1), 0)));
      expect(res.status).toBe(200);
      expect(res.body.results).toHaveLength(100);
      expect(res.body.results.every((r: { status: string }) => r.status === "ok")).toBe(true);
    });

    it("rejects 101 changes with 413 and writes nothing", async () => {
      const token = await loggedIn();
      const res = await push(token, Array.from({ length: 101 }, (_, i) => change(uuid(i + 1), 0)));
      expect(res.status).toBe(413);
      expect(res.body.error).toBe("too_many_changes");
      const count = await env.DB.prepare("SELECT COUNT(*) AS n FROM items").first<{ n: number }>();
      expect(count!.n).toBe(0);
    });

    it("accepts a 64 KB envelope but flags a larger one per item without blocking the others", async () => {
      const token = await loggedIn();
      const exactly = "a".repeat(MAX_ENVELOPE_BYTES);
      const tooBig = "a".repeat(MAX_ENVELOPE_BYTES + 1);
      const res = await push(token, [
        { id: ID_1, base_revision: 0, deleted: false, envelope: exactly },
        { id: ID_2, base_revision: 0, deleted: false, envelope: tooBig },
        change(ID_3, 0),
      ]);
      expect(res.status).toBe(200);
      expect(res.body.results).toEqual([
        { id: ID_1, status: "ok", revision: 1, seq: 1 },
        { id: ID_2, status: "error", error: "too_large" },
        { id: ID_3, status: "ok", revision: 1, seq: 2 },
      ]);
    });

    it("counts UTF-8 bytes, not characters, for the envelope limit", async () => {
      const token = await loggedIn();
      const multibyte = "€".repeat(Math.floor(MAX_ENVELOPE_BYTES / 3) + 1); // 3 bytes per char
      const res = await push(token, [{ id: ID_1, base_revision: 0, deleted: false, envelope: multibyte }]);
      expect(res.body.results[0]).toEqual({ id: ID_1, status: "error", error: "too_large" });
    });

    it("rejects a request body beyond the overall size ceiling with 413", async () => {
      const token = await loggedIn();
      const res = await call("/v1/items", { token, body: `{"changes":[],"pad":"${"x".repeat(9 * 1024 * 1024)}"}`, headers: { "Content-Type": "application/json" } });
      expect(res.status).toBe(413);
      expect(res.body.error).toBe("payload_too_large");
    });
  });

  describe("validation", () => {
    it.each([
      ["changes missing", {}],
      ["changes not an array", { changes: "nope" }],
      ["change not an object", { changes: [1] }],
      ["id missing", { changes: [{ base_revision: 0, deleted: false, envelope: "x" }] }],
      ["id with illegal characters", { changes: [{ id: "a/b", base_revision: 0, deleted: false, envelope: "x" }] }],
      ["id too long", { changes: [{ id: "a".repeat(65), base_revision: 0, deleted: false, envelope: "x" }] }],
      ["base_revision negative", { changes: [{ id: "a", base_revision: -1, deleted: false, envelope: "x" }] }],
      ["base_revision not an integer", { changes: [{ id: "a", base_revision: 1.5, deleted: false, envelope: "x" }] }],
      ["base_revision missing", { changes: [{ id: "a", deleted: false, envelope: "x" }] }],
      ["deleted not a boolean", { changes: [{ id: "a", base_revision: 0, deleted: 0, envelope: "x" }] }],
      ["live item without envelope", { changes: [{ id: "a", base_revision: 0, deleted: false, envelope: null }] }],
      ["live item with empty envelope", { changes: [{ id: "a", base_revision: 0, deleted: false, envelope: "" }] }],
      ["envelope not a string", { changes: [{ id: "a", base_revision: 0, deleted: false, envelope: { v: 1 } }] }],
      ["deleted item with an envelope", { changes: [{ id: "a", base_revision: 1, deleted: true, envelope: "x" }] }],
      ["updated_at negative", { changes: [{ id: "a", base_revision: 0, deleted: false, envelope: "x", updated_at: -5 }] }],
      ["updated_at not an integer", { changes: [{ id: "a", base_revision: 0, deleted: false, envelope: "x", updated_at: "now" }] }],
      [
        "duplicate ids",
        {
          changes: [
            { id: "a", base_revision: 0, deleted: false, envelope: "x" },
            { id: "a", base_revision: 1, deleted: false, envelope: "y" },
          ],
        },
      ],
    ])("rejects the whole request with 400: %s", async (_name, body) => {
      const token = await loggedIn();
      const res = await call("/v1/items", { token, json: body });
      expect(res.status).toBe(400);
      expect(res.body.error).toBe("invalid_request");
      const count = await env.DB.prepare("SELECT COUNT(*) AS n FROM items").first<{ n: number }>();
      expect(count!.n).toBe(0);
    });

    it("writes nothing when a later change in the request is invalid", async () => {
      const token = await loggedIn();
      const res = await push(token, [change(ID_1, 0), { id: "bad id", base_revision: 0, deleted: false, envelope: "x" }]);
      expect(res.status).toBe(400);
      expect(res.body.message).toContain("changes[1]");
      const count = await env.DB.prepare("SELECT COUNT(*) AS n FROM items").first<{ n: number }>();
      expect(count!.n).toBe(0);
    });

    it("rejects non-JSON bodies with 415", async () => {
      const token = await loggedIn();
      const res = await call("/v1/items", { token, method: "POST", body: "changes", headers: { "Content-Type": "text/plain" } });
      expect(res.status).toBe(415);
    });
  });
});

describe("GET /v1/items", () => {
  const seed = async (token: string, count: number) => {
    const res = await push(token, Array.from({ length: count }, (_, i) => change(uuid(i + 1), 0, `item-${i + 1}`)));
    expect(res.status).toBe(200);
  };

  it("returns an empty page when there is nothing to pull", async () => {
    const token = await loggedIn();
    const res = await call("/v1/items", { token });
    expect(res.status).toBe(200);
    expect(res.body).toEqual({ items: [], next_since: 0, has_more: false });
  });

  it("returns items ordered by seq with the documented shape", async () => {
    const token = await loggedIn();
    await push(token, [change(ID_2, 0, "b", { updated_at: 222 }), change(ID_1, 0, "a", { updated_at: 111 })]);
    await push(token, [change(ID_2, 1, "b2", { updated_at: 333 })]);

    const res = await call("/v1/items", { token });
    expect(res.body.has_more).toBe(false);
    expect(res.body.next_since).toBe(3);
    expect(res.body.items).toEqual([
      { id: ID_1, envelope: envelope("a"), revision: 1, seq: 2, deleted: false, updated_at: 111 },
      { id: ID_2, envelope: envelope("b2"), revision: 2, seq: 3, deleted: false, updated_at: 333 },
    ]);
  });

  it("paginates with has_more and next_since until exhausted", async () => {
    const token = await loggedIn();
    await seed(token, 5);

    const page1 = await call("/v1/items?since=0&limit=2", { token });
    expect(page1.body.items.map((i: { seq: number }) => i.seq)).toEqual([1, 2]);
    expect(page1.body).toMatchObject({ next_since: 2, has_more: true });

    const page2 = await call(`/v1/items?since=${page1.body.next_since}&limit=2`, { token });
    expect(page2.body.items.map((i: { seq: number }) => i.seq)).toEqual([3, 4]);
    expect(page2.body).toMatchObject({ next_since: 4, has_more: true });

    const page3 = await call(`/v1/items?since=${page2.body.next_since}&limit=2`, { token });
    expect(page3.body.items.map((i: { seq: number }) => i.seq)).toEqual([5]);
    expect(page3.body).toMatchObject({ next_since: 5, has_more: false });

    const page4 = await call(`/v1/items?since=${page3.body.next_since}&limit=2`, { token });
    expect(page4.body).toEqual({ items: [], next_since: 5, has_more: false });
  });

  it("reports has_more=false when the last page is exactly full", async () => {
    const token = await loggedIn();
    await seed(token, 4);
    const res = await call("/v1/items?limit=4", { token });
    expect(res.body.items).toHaveLength(4);
    expect(res.body.has_more).toBe(false);
  });

  it("returns only changes after the cursor, including updates and deletions", async () => {
    const token = await loggedIn();
    await seed(token, 3);
    const cursor = (await call("/v1/items", { token })).body.next_since;

    await push(token, [change(uuid(1), 1, "edited"), { id: uuid(2), base_revision: 1, deleted: true, envelope: null }]);
    const res = await call(`/v1/items?since=${cursor}`, { token });
    expect(res.body.items.map((i: { id: string; deleted: boolean }) => [i.id, i.deleted])).toEqual([
      [uuid(1), false],
      [uuid(2), true],
    ]);
    expect(res.body.items.every((i: { seq: number }) => i.seq > cursor)).toBe(true);
  });

  it("defaults to 500 items per page and clamps limit to 1000", async () => {
    const token = await loggedIn();
    const rows = Array.from({ length: 1005 }, (_, i) => env.DB.prepare(
      "INSERT INTO items (id, envelope, revision, seq, deleted, updated_at) VALUES (?, 'e', 1, ?, 0, 0)",
    ).bind(`bulk-${i}`, i + 1));
    for (let i = 0; i < rows.length; i += 200) await env.DB.batch(rows.slice(i, i + 200));

    const dflt = await call("/v1/items", { token });
    expect(dflt.body.items).toHaveLength(500);
    expect(dflt.body).toMatchObject({ has_more: true, next_since: 500 });

    const clamped = await call("/v1/items?limit=5000", { token });
    expect(clamped.body.items).toHaveLength(1000);
    expect(clamped.body).toMatchObject({ has_more: true, next_since: 1000 });
  });

  it.each([
    ["since=-1"],
    ["since=abc"],
    ["since=1.5"],
    ["since="],
    ["limit=0"],
    ["limit=-3"],
    ["limit=ten"],
  ])("rejects bad query parameters with 400: %s", async (query) => {
    const token = await loggedIn();
    const res = await call(`/v1/items?${query}`, { token });
    expect(res.status).toBe(400);
    expect(res.body.error).toBe("invalid_request");
  });
});
