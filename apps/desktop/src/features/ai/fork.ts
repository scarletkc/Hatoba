import { LOCALES, translate, type MessageKey, type Params } from "@/i18n";
import type { AiEntryView, AiModelRef } from "@/ipc/types";
import { lastModel, sameModel } from "./models";

type Tr = (key: MessageKey, params?: Params) => string;

/** The most characters a conversation's title holds, as Rust's `MAX_TITLE_CHARS` (AI-23): a rename and a fork's title. */
export const MAX_TITLE_CHARS = 200;

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
 * AI-43: the entries a fork from the entry at `at` copies, by index, in order, like Rust's `fork_copies`:
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

/** The entries a fork may start from (`forkCopies`), by ID: where the panel offers Fork from Here (AI-43). */
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
 * AI-43: the model a fork opens with in the selector, or null to follow its newest copied reply (AI-05). A
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

/** A title's fork marks, numbered or not, in every language a device may have marked it in (`forkTitle`). */
const FORK_MARKS = LOCALES.flatMap((locale) => {
  const escape = (text: string) => text.trim().replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const numbered = escape(translate(locale, "ai.fork.titleNumbered", { title: "", n: "#" })).replace("#", "\\d+");
  return [new RegExp(`\\s*${escape(translate(locale, "ai.fork.title", { title: "" }))}$`), new RegExp(`\\s*${numbered}$`)];
});

/**
 * AI-43: a fork's title in the panel's language: the original's without the fork marks it ends with, in any
 * language, then "(fork)", or "(fork 2)", "(fork 3)"… for the first that no title in `taken` (History's) has,
 * with the original's title cut so the whole fits in a title.
 */
export function forkTitle(t: Tr, title: string, taken: Iterable<string>): string {
  const base = withoutMarks(title.trim()) || t("ai.untitled");
  const used = new Set(taken);
  const titled = (title: string, n: number) => (n === 1 ? t("ai.fork.title", { title }) : t("ai.fork.titleNumbered", { title, n }));
  for (let n = 1; ; n++) {
    const room = MAX_TITLE_CHARS - [...titled("", n)].length;
    const candidate = titled([...base].slice(0, room).join("").trimEnd(), n);
    if (!used.has(candidate)) return candidate;
  }
}

function withoutMarks(title: string): string {
  const mark = FORK_MARKS.find((m) => m.test(title));
  return mark ? withoutMarks(title.replace(mark, "")) : title;
}
