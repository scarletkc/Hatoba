import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiConversationDetail, AiConversationView, AiEntryView } from "@/ipc/types";
import { newTurn } from "./turn";

const fork = vi.fn<(id: string, entryId: string | null, title: string) => Promise<AiConversationView>>();
const get = vi.fn<(id: string) => Promise<AiConversationDetail>>();
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: { ai_conversation_fork: (id: string, entryId: string | null, title: string) => fork(id, entryId, title), ai_conversation_get: (id: string) => get(id) },
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

beforeEach(() => {
  fork.mockReset();
  get.mockReset();
  toast.mockReset();
  get.mockResolvedValue({ conversation: copy, entries: [entry], running: false });
  useAi.setState({ history: null, slots: { [HOME_SLOT]: { ...blankSlot("manual"), conversationId: original.id, conversation: original, entries: [entry] } } });
});

describe("forkConversation (AI-41)", () => {
  it("opens the fork in the slot", async () => {
    fork.mockResolvedValue(copy);
    await forkConversation(HOME_SLOT, original.id, entry.entry_id);
    expect(fork).toHaveBeenCalledWith(original.id, entry.entry_id, "Disk usage (fork)");
    expect(getSlot(HOME_SLOT).conversationId).toBe(copy.id);
    expect(toast).toHaveBeenCalledWith("Forked. The original conversation stays in History.", "success");
  });

  it("leaves a slot that moved on while Rust copied, and says the fork is in History", async () => {
    let finish!: (view: AiConversationView) => void;
    fork.mockReturnValue(new Promise((resolve) => (finish = resolve)));
    const pending = forkConversation(HOME_SLOT, original.id, null);
    patchSlot(HOME_SLOT, { conversationId: "c3", conversation: conversation("c3", "Other") });
    finish(copy);
    await pending;
    expect(getSlot(HOME_SLOT).conversationId).toBe("c3");
    expect(get).not.toHaveBeenCalled();
    expect(toast).toHaveBeenCalledWith("Forked. The new conversation is in History.", "success");
  });

  it("does nothing while the conversation is busy", async () => {
    patchSlot(HOME_SLOT, { turn: newTurn() });
    await forkConversation(HOME_SLOT, original.id, null);
    patchSlot(HOME_SLOT, { turn: null, compacting: true });
    await forkConversation(HOME_SLOT, original.id, null);
    expect(fork).not.toHaveBeenCalled();
  });
});
