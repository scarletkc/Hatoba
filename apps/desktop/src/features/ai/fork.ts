import type { MessageKey, Params } from "@/i18n";
import type { AiEntryView, AiModelRef } from "@/ipc/types";
import { lastModel, sameModel } from "./models";

type Tr = (key: MessageKey, params?: Params) => string;

/**
 * For each tool call, keyed `entry index:call index`, the index of the result that answers it: the first
 * result with its ID stored after the call that no earlier call took, as Rust's `pair_results` pairs them.
 */
function pairResults(entries: AiEntryView[]): Map<string, number> {
  const results = new Map<string, number[]>();
  for (const [i, e] of entries.entries()) {
    if (e.role !== "tool") continue;
    const queue = results.get(e.tool_call_id);
    if (queue) queue.push(i);
    else results.set(e.tool_call_id, [i]);
  }
  const pairs = new Map<string, number>();
  for (const [i, e] of entries.entries()) {
    if (e.role !== "assistant") continue;
    for (const [c, call] of e.tool_calls.entries()) {
      const queue = results.get(call.id);
      while (queue?.length && queue[0] < i) queue.shift();
      const answer = queue?.shift();
      if (answer !== undefined) pairs.set(`${i}:${c}`, answer);
    }
  }
  return pairs;
}

/**
 * AI-41: the entries a fork from the entry at `at` copies, by index, in order, like Rust's `fork_copies`:
 * every entry up to it, it included, and each tool result stored after it that answers a call of a copied
 * reply, so no copied call loses its result. null when `at` is no fork point: one of the user's stored
 * messages, or a reply entry whose every call has a result.
 */
export function forkCopies(entries: AiEntryView[], at: number): number[] | null {
  const entry = entries[at];
  if (!entry || entry.entry_id.startsWith("~") || (entry.role !== "user" && entry.role !== "assistant")) return null;
  const pairs = pairResults(entries);
  if (entry.role === "assistant" && entry.tool_calls.some((_, c) => !pairs.has(`${at}:${c}`))) return null;
  const late = [...pairs]
    .filter(([key, result]) => Number(key.split(":")[0]) <= at && result > at)
    .map(([, result]) => result)
    .sort((a, b) => a - b);
  return [...entries.keys()].filter((i) => i <= at).concat(late);
}

/** The entries a fork may start from (`forkCopies`), by ID: where the panel offers Fork from Here (AI-41). */
export function forkPoints(entries: AiEntryView[]): Set<string> {
  const pairs = pairResults(entries);
  const points = new Set<string>();
  for (const [i, e] of entries.entries()) {
    if (e.entry_id.startsWith("~")) continue;
    if (e.role === "user" || (e.role === "assistant" && e.tool_calls.every((_, c) => pairs.has(`${i}:${c}`)))) points.add(e.entry_id);
  }
  return points;
}

/**
 * AI-41: the model a fork opens with in the selector, or null to follow its newest copied reply (AI-05). A
 * user message that the original's next reply answered with another model goes on with that model.
 */
export function forkModel(entries: AiEntryView[], entryId: string): AiModelRef | null {
  const at = entries.findIndex((e) => e.entry_id === entryId);
  if (entries[at]?.role !== "user") return null;
  const reply = entries.slice(at + 1).find((e) => e.role === "user" || e.role === "assistant");
  if (reply?.role !== "assistant") return null;
  const answered = { provider_id: reply.provider_id, model_id: reply.model_id };
  return sameModel(answered, lastModel(entries.slice(0, at + 1))) ? null : answered;
}

/** AI-41: a fork's title, "*title* (fork)" in the panel's language; a fork of a fork keeps its title rather than marking it twice. */
export function forkTitle(t: Tr, title: string): string {
  const mark = t("ai.fork.title", { title: "" }).trim();
  const original = title.trim() || t("ai.untitled");
  return original.endsWith(mark) ? original : t("ai.fork.title", { title: original });
}
