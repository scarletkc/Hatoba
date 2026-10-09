import { Hono } from "hono";
import {
  DEFAULT_PULL_LIMIT,
  MAX_BODY_BYTES_PUSH,
  MAX_CHANGES_PER_PUSH,
  MAX_ENVELOPE_BYTES,
  MAX_PULL_LIMIT,
} from "../config";
import type { AppEnv } from "../env";
import { ApiError } from "../errors";
import { limitBody, requireSession } from "../middleware";
import { exceedsBytes } from "../util";
import { invalid, isObject, readBoolean, readId, readInteger, readJsonBody, type JsonObject } from "../validate";

export const itemsRoutes = new Hono<AppEnv>();

interface ItemRow {
  id: string;
  envelope: string | null;
  revision: number;
  seq: number;
  deleted: number;
  updated_at: number;
}

interface ServerItem {
  revision: number;
  seq: number;
  deleted: boolean;
  envelope: string | null;
  updated_at: number;
}

type PushResult =
  | { id: string; status: "ok"; revision: number; seq: number }
  | { id: string; status: "conflict"; server: ServerItem }
  | { id: string; status: "error"; error: "too_large" | "not_found" };

interface Change {
  id: string;
  baseRevision: number;
  deleted: boolean;
  envelope: string | null;
  updatedAt: number | null;
}

const ITEM_COLUMNS = "id, envelope, revision, seq, deleted, updated_at";

const toServerItem = (row: ItemRow): ServerItem => ({
  revision: row.revision,
  seq: row.seq,
  deleted: row.deleted === 1,
  envelope: row.envelope,
  updated_at: row.updated_at,
});

function readQueryInteger(raw: string | undefined, name: string, fallback: number, min: number): number {
  if (raw === undefined) return fallback;
  if (!/^\d{1,15}$/.test(raw)) throw invalid(name, "must be a non-negative integer");
  const value = Number(raw);
  if (value < min) throw invalid(name, `must be at least ${min}`);
  return value;
}

/**
 * Incremental pull: items with `seq > since`, ordered by `seq`. One extra row is fetched to
 * learn whether more pages exist; `limit` above the maximum is clamped.
 */
itemsRoutes.get("/items", requireSession(), async (c) => {
  const since = readQueryInteger(c.req.query("since"), "since", 0, 0);
  const limit = Math.min(readQueryInteger(c.req.query("limit"), "limit", DEFAULT_PULL_LIMIT, 1), MAX_PULL_LIMIT);

  const { results } = await c.env.DB.prepare(
    `SELECT ${ITEM_COLUMNS} FROM items WHERE seq > ? ORDER BY seq LIMIT ?`,
  )
    .bind(since, limit + 1)
    .all<ItemRow>();

  const hasMore = results.length > limit;
  const page = hasMore ? results.slice(0, limit) : results;
  return c.json({
    items: page.map((row) => ({ id: row.id, ...toServerItem(row) })),
    next_since: page.at(-1)?.seq ?? since,
    has_more: hasMore,
  });
});

function parseChange(raw: unknown): Change {
  if (!isObject(raw)) throw invalid("change", "must be an object");
  const deleted = readBoolean(raw, "deleted");
  const envelope = raw.envelope ?? null;
  if (deleted) {
    if (envelope !== null) throw invalid("envelope", "must be null for deleted items");
  } else if (typeof envelope !== "string" || envelope.length === 0) {
    throw invalid("envelope", "must be a non-empty string unless the item is deleted");
  }
  return {
    id: readId(raw, "id"),
    baseRevision: readInteger(raw, "base_revision", { min: 0 }),
    deleted,
    envelope,
    updatedAt: raw.updated_at === undefined || raw.updated_at === null ? null : readInteger(raw, "updated_at", { min: 0 }),
  };
}

/**
 * Validates the whole request before anything is written. Structural problems reject the
 * request (400/413); an oversized envelope is a per-change `too_large` result instead, so one
 * bad item can never block the rest of a client's push queue.
 */
function parseChanges(body: JsonObject): Change[] {
  const raw = body.changes;
  if (!Array.isArray(raw)) throw invalid("changes", "must be an array");
  if (raw.length > MAX_CHANGES_PER_PUSH) {
    throw new ApiError(413, "too_many_changes", `At most ${MAX_CHANGES_PER_PUSH} changes per request`);
  }
  const seen = new Set<string>();
  return raw.map((entry, index) => {
    try {
      const change = parseChange(entry);
      if (seen.has(change.id)) throw invalid("id", "appears more than once in the request");
      seen.add(change.id);
      return change;
    } catch (err) {
      if (err instanceof ApiError && err.status === 400) throw invalid(`changes[${index}]`, err.message);
      throw err;
    }
  });
}

/**
 * Applies one change as a single D1 batch (a transaction): bump the global sequence, then
 * insert (`base_revision = 0`) or compare-and-swap update (`revision = base_revision`).
 * RETURNING yields the new revision and seq; no row means the optimistic check lost.
 * A lost race still consumed a seq number - gaps are allowed, only monotonicity matters.
 *
 * The bump starts from the larger of `meta.seq` and the highest item `seq`: a client in D1
 * direct mode writes items in single statements and catches `meta.seq` up only afterwards, so
 * `meta.seq` alone could hand out a seq that an item already has, and a client whose pull cursor
 * is past it would never see this change. D1 direct mode allocates the same way.
 */
async function applyChange(db: D1Database, change: Change, now: number): Promise<PushResult> {
  const { id, baseRevision, envelope } = change;
  const deleted = change.deleted ? 1 : 0;
  const updatedAt = change.updatedAt ?? now;

  const bumpSeq = db.prepare(
    "UPDATE meta SET seq = MAX(seq, (SELECT COALESCE(MAX(seq), 0) FROM items)) + 1 WHERE id = 1",
  );
  const write =
    baseRevision === 0
      ? db
          .prepare(
            `INSERT INTO items (id, envelope, revision, seq, deleted, updated_at)
             VALUES (?, ?, 1, (SELECT seq FROM meta WHERE id = 1), ?, ?)
             ON CONFLICT(id) DO NOTHING
             RETURNING revision, seq`,
          )
          .bind(id, envelope, deleted, updatedAt)
      : db
          .prepare(
            `UPDATE items
             SET envelope = ?, deleted = ?, revision = revision + 1,
                 seq = (SELECT seq FROM meta WHERE id = 1), updated_at = ?
             WHERE id = ? AND revision = ?
             RETURNING revision, seq`,
          )
          .bind(envelope, deleted, updatedAt, id, baseRevision);

  const [, writeResult] = await db.batch<{ revision: number; seq: number }>([bumpSeq, write]);
  const applied = writeResult?.results[0];
  if (applied) return { id, status: "ok", revision: applied.revision, seq: applied.seq };

  const current = await db.prepare(`SELECT ${ITEM_COLUMNS} FROM items WHERE id = ?`).bind(id).first<ItemRow>();
  // base_revision > 0 for an item the server has never seen (e.g. the database was reset).
  if (!current) return { id, status: "error", error: "not_found" };
  return { id, status: "conflict", server: toServerItem(current) };
}

itemsRoutes.post("/items", requireSession(), limitBody(MAX_BODY_BYTES_PUSH), async (c) => {
  const changes = parseChanges(await readJsonBody(c));
  const now = Date.now();

  // Sequential on purpose: results keep request order and seq order matches it.
  const results: PushResult[] = [];
  for (const change of changes) {
    if (change.envelope !== null && exceedsBytes(change.envelope, MAX_ENVELOPE_BYTES)) {
      results.push({ id: change.id, status: "error", error: "too_large" });
    } else {
      results.push(await applyChange(c.env.DB, change, now));
    }
  }
  return c.json({ results });
});
