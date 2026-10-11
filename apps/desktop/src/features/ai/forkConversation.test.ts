import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiConversationDetail, AiConversationView, AiEntryView } from "@/ipc/types";
import { newTurn } from "./turn";

const fork = vi.fn<(id: string, entryId: string | null, title: string) => Promise<AiConversationView>>();
const get = vi.fn<(id: string) => Promise<AiConversationDetail>>();
const list = vi.fn<() => Promise<AiConversationView[]>>();
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    ai_conversation_fork: (id: string, entryId: string | null, title: string) => fork(id, entryId, title),
    ai_conversation_get: (id: string) => get(id),
    ai_conversations_list: () => list(),
  },
}));
const toast = vi.fn();
vi.mock("@/components/overlay", async (importActual) => ({ ...(await importActual<typeof import("@/components/overlay")>()), toast: (...args: unknown[]) => toast(...args) }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { forkConversation } = await import("./actions");
const { blankSlot, getSlot, HOME_SLOT, patchSlot, useAi } = await import("./store");

const conversation = (id: string, title: string): AiConversationView => ({
  id,
  title,
  host_id: null,
  quick_target: null,
  origin: null,
  pinned: false,
  context_start: null,
  effort: null,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
});
const original = conversation("c1", "Disk usage");
const copy = conversation("c2", "Disk usage (fork)");
const entry: AiEntryView = { role: "user", entry_id: "e1", created_at: 0, text: "How full is the disk?" };

/** Makes the next fork call return once the function it returns is called. */
function held() {
  let finish!: (view: AiConversationView) => void;
  fork.mockReturnValue(new Promise((resolve) => (finish = resolve)));
  return (view: AiConversationView) => finish(view);
}

beforeEach(() => {
  fork.mockReset();
  get.mockReset();
  list.mockReset();
  toast.mockReset();
  get.mockResolvedValue({ conversation: copy, entries: [entry], running: false, file_changes: [] });
  list.mockResolvedValue([original]);
  useAi.setState({ history: null, forking: [], slots: { [HOME_SLOT]: { ...blankSlot("manual"), conversationId: original.id, conversation: original, entries: [entry] } } });
});

describe("forkConversation (AI-43)", () => {
  it("opens the fork in the slot", async () => {
    fork.mockResolvedValue(copy);
    await forkConversation(HOME_SLOT, original.id, entry.entry_id);
    expect(fork).toHaveBeenCalledWith(original.id, entry.entry_id, "Disk usage (fork)");
    expect(getSlot(HOME_SLOT).conversationId).toBe(copy.id);
    expect(toast).toHaveBeenCalledWith("Forked. The original conversation stays in History.", "success");
  });

  it("opens the fork in a tab that never showed a conversation", async () => {
    useAi.setState({ history: [original], slots: {} });
    fork.mockResolvedValue(copy);
    await forkConversation(HOME_SLOT, original.id, null);
    expect(getSlot(HOME_SLOT).conversationId).toBe(copy.id);
    expect(toast).toHaveBeenCalledWith("Forked. The original conversation stays in History.", "success");
  });

  it("numbers the title past those History has", async () => {
    list.mockResolvedValue([original, copy]);
    fork.mockResolvedValue(conversation("c3", "Disk usage (fork 2)"));
    await forkConversation(HOME_SLOT, original.id, null);
    expect(list).toHaveBeenCalledOnce();
    expect(fork).toHaveBeenCalledWith(original.id, null, "Disk usage (fork 2)");
  });

  it("leaves a slot that moved on while Rust copied, and says the fork is in History", async () => {
    const finish = held();
    const pending = forkConversation(HOME_SLOT, original.id, null);
    patchSlot(HOME_SLOT, { conversationId: "c3", conversation: conversation("c3", "Other") });
    finish(copy);
    await pending;
    expect(getSlot(HOME_SLOT).conversationId).toBe("c3");
    expect(get).not.toHaveBeenCalled();
    expect(toast).toHaveBeenCalledWith("Forked. The new conversation is in History.", "success");
  });

  it("leaves a slot where a turn started and ended while Rust copied", async () => {
    const finish = held();
    const pending = forkConversation(HOME_SLOT, original.id, null);
    const reply: AiEntryView = { ...entry, entry_id: "e2", text: "And now?" };
    patchSlot(HOME_SLOT, (s) => ({ entries: [...s.entries, reply], turn: newTurn() }));
    patchSlot(HOME_SLOT, { turn: null });
    finish(copy);
    await pending;
    expect(getSlot(HOME_SLOT).conversationId).toBe(original.id);
    expect(toast).toHaveBeenCalledWith("Forked. The new conversation is in History.", "success");
  });

  it("leaves a slot whose turn started while Rust copied and still runs", async () => {
    const finish = held();
    const pending = forkConversation(HOME_SLOT, original.id, null);
    patchSlot(HOME_SLOT, { turn: newTurn() });
    finish(copy);
    await pending;
    expect(getSlot(HOME_SLOT).conversationId).toBe(original.id);
    expect(getSlot(HOME_SLOT).turn).not.toBeNull();
    expect(toast).toHaveBeenCalledWith("Forked. The new conversation is in History.", "success");
  });

  it("forks from History without stopping the active tab's turn that waits for approval", async () => {
    const other = conversation("c3", "Restart nginx");
    const waiting = { ...newTurn(), phase: "tools" as const, call: { id: "call-1", state: "approval" as const } };
    useAi.setState({ history: [original, other], slots: { [HOME_SLOT]: { ...blankSlot("manual"), conversationId: other.id, conversation: other, turn: waiting } } });
    fork.mockResolvedValue(copy);
    await forkConversation(HOME_SLOT, original.id, null);
    expect(fork).toHaveBeenCalledWith(original.id, null, "Disk usage (fork)");
    expect(getSlot(HOME_SLOT).conversationId).toBe(other.id);
    expect(getSlot(HOME_SLOT).turn).toBe(waiting);
    expect(get).not.toHaveBeenCalled();
    expect(useAi.getState().history?.map((c) => c.id)).toEqual([copy.id, original.id, other.id]);
    expect(toast).toHaveBeenCalledWith("Forked. The new conversation is in History.", "success");
  });

  it("makes one fork of a conversation at a time, so a double click makes one copy", async () => {
    const finish = held();
    const first = forkConversation(HOME_SLOT, original.id, entry.entry_id);
    expect(useAi.getState().forking).toEqual([original.id]);
    await forkConversation(HOME_SLOT, original.id, entry.entry_id);
    await forkConversation(HOME_SLOT, original.id, null);
    finish(copy);
    await first;
    expect(fork).toHaveBeenCalledOnce();
    expect(useAi.getState().forking).toEqual([]);
    // A failed fork lets the next one go.
    fork.mockRejectedValue({ code: "internal", message: "disk full" });
    await forkConversation(HOME_SLOT, copy.id, null);
    expect(useAi.getState().forking).toEqual([]);
    expect(toast).toHaveBeenLastCalledWith(expect.any(String), "error");
  });

  it("does nothing while the conversation is busy", async () => {
    patchSlot(HOME_SLOT, { turn: newTurn() });
    await forkConversation(HOME_SLOT, original.id, null);
    patchSlot(HOME_SLOT, { turn: null, compacting: true });
    await forkConversation(HOME_SLOT, original.id, null);
    expect(fork).not.toHaveBeenCalled();
  });
});
