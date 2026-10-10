import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AiConversationView, AiEntryView } from "@/ipc/types";
import type { PasteAttachment } from "./attachments";
import { newTurn } from "./turn";

// The action never calls the backend; this keeps the real IPC layer out of the test.
vi.mock("@/ipc/api", async (importActual) => ({ ...(await importActual<typeof import("@/ipc/api")>()), api: {} }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { askAiAboutConnection, continueHomeConversation, noteSelection } = await import("./actions");
const { blankSlot, getSlot, HOME_SLOT, offersHomeConversation, patchSlot, useAi } = await import("./store");
type Slot = import("./store").Slot;

const TAB = "tab-1";
const view: AiConversationView = {
  id: "c1",
  title: "nginx 502",
  host_id: null,
  quick_target: null,
  pinned: false,
  context_start: null,
  effort: null,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
};
const entry: AiEntryView = { role: "user", entry_id: "e1", created_at: 0, text: "Why does nginx return 502?" };
const paste: PasteAttachment = { kind: "paste", text: "log", lines: 1 };

/** The home tab with a stored conversation; `patch` changes it. */
function homeSlot(patch: Partial<Slot> = {}): Slot {
  return { ...blankSlot("manual"), conversationId: view.id, conversation: view, entries: [entry], ...patch };
}

function setSlots(home: Slot | null, tab: Slot = blankSlot("manual")) {
  useAi.setState({ slots: { ...(home ? { [HOME_SLOT]: home } : {}), [TAB]: tab }, selections: {}, diagnostics: {} });
}

beforeEach(() => setSlots(homeSlot()));

describe("offersHomeConversation (AI-09)", () => {
  it("offers an idle home conversation with messages to a new, empty conversation", () => {
    expect(offersHomeConversation(blankSlot("manual"), homeSlot())).toBe(true);
  });

  it("does not offer it to a tab that already has a conversation or a message on its way", () => {
    expect(offersHomeConversation({ ...blankSlot("manual"), conversationId: "c2" }, homeSlot())).toBe(false);
    expect(offersHomeConversation({ ...blankSlot("manual"), entries: [entry], turn: newTurn() }, homeSlot())).toBe(false);
  });

  it("does not offer a home tab without a stored conversation with messages", () => {
    expect(offersHomeConversation(blankSlot("manual"), undefined)).toBe(false);
    expect(offersHomeConversation(blankSlot("manual"), blankSlot("manual"))).toBe(false);
    expect(offersHomeConversation(blankSlot("manual"), homeSlot({ entries: [], loading: true }))).toBe(false);
  });

  it("waits while the home conversation runs a turn, waits for approval, or compacts", () => {
    expect(offersHomeConversation(blankSlot("manual"), homeSlot({ turn: newTurn() }))).toBe(false);
    expect(offersHomeConversation(blankSlot("manual"), homeSlot({ remoteRunning: true }))).toBe(false);
    expect(offersHomeConversation(blankSlot("manual"), homeSlot({ compacting: true }))).toBe(false);
  });
});

describe("continueHomeConversation (AI-09)", () => {
  it("attaches the home conversation to the tab and gives the home tab a new one", () => {
    setSlots(homeSlot({ mode: "bypass", mcpOff: ["fs"] }));
    expect(continueHomeConversation(TAB)).toBe(true);
    const tab = getSlot(TAB);
    expect(tab.conversationId).toBe(view.id);
    expect(tab.entries).toEqual([entry]);
    expect(tab.mode).toBe("bypass");
    expect(tab.mcpOff).toEqual(["fs"]);
    const home = getSlot(HOME_SLOT);
    expect(home.conversationId).toBeNull();
    expect(home.entries).toEqual([]);
  });

  it("brings the home tab's unsent text and attachments when the tab's input is empty", () => {
    setSlots(homeSlot({ draft: "and the upstream?", extras: [{ id: "x1", attachment: paste }] }));
    continueHomeConversation(TAB);
    expect(getSlot(TAB)).toMatchObject({ draft: "and the upstream?", extras: [{ id: "x1", attachment: paste }] });
    expect(getSlot(HOME_SLOT)).toMatchObject({ draft: "", extras: [] });
  });

  it("keeps each input where it is when the tab's input holds something", () => {
    setSlots(homeSlot({ draft: "home draft", extras: [{ id: "x1", attachment: paste }] }), { ...blankSlot("manual"), draft: "tab draft" });
    continueHomeConversation(TAB);
    expect(getSlot(TAB)).toMatchObject({ conversationId: view.id, draft: "tab draft", extras: [] });
    expect(getSlot(HOME_SLOT)).toMatchObject({ conversationId: null, draft: "home draft", extras: [{ id: "x1", attachment: paste }] });
  });

  it("counts the tab's selection and diagnostics chips as input it holds (AI-10)", () => {
    const attach = [
      () => noteSelection(TAB, "502 Bad Gateway"),
      () => {
        askAiAboutConnection(TAB, "prod-api", "kind: timeout");
        patchSlot(TAB, { draft: "" }); // leave only the chip, without the suggested question
      },
    ];
    for (const chip of attach) {
      setSlots(homeSlot({ draft: "home draft" }));
      chip();
      continueHomeConversation(TAB);
      expect(getSlot(TAB)).toMatchObject({ conversationId: view.id, draft: "" });
      expect(getSlot(HOME_SLOT)).toMatchObject({ conversationId: null, draft: "home draft" });
    }
  });

  it("leaves a busy home conversation where it is", () => {
    for (const busy of [{ turn: newTurn() }, { remoteRunning: true }, { compacting: true }]) {
      setSlots(homeSlot(busy));
      expect(continueHomeConversation(TAB)).toBe(false);
      expect(getSlot(HOME_SLOT).conversationId).toBe(view.id);
      expect(getSlot(TAB).conversationId).toBeNull();
    }
  });

  it("never replaces the tab's own conversation", () => {
    setSlots(homeSlot(), { ...blankSlot("manual"), conversationId: "c2", entries: [{ ...entry, entry_id: "e2" }] });
    expect(continueHomeConversation(TAB)).toBe(false);
    expect(getSlot(TAB).conversationId).toBe("c2");
    expect(getSlot(HOME_SLOT).conversationId).toBe(view.id);
  });

  it("does nothing without a home conversation, or for the home tab itself", () => {
    setSlots(null);
    expect(continueHomeConversation(TAB)).toBe(false);
    expect(getSlot(TAB).conversationId).toBeNull();
    setSlots(homeSlot());
    expect(continueHomeConversation(HOME_SLOT)).toBe(false);
    expect(getSlot(HOME_SLOT).conversationId).toBe(view.id);
  });
});
