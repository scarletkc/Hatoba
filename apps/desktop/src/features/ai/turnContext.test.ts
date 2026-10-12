import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionTab } from "@/app/tabs";
import type { AiConversationView, AiProviderView, AiSettingsView, QuickTarget } from "@/ipc/types";

// turnContext never calls the backend; this keeps the real IPC layer out of the test.
vi.mock("@/ipc/api", async (importActual) => ({ ...(await importActual<typeof import("@/ipc/api")>()), api: {} }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
// The quick-connect tab's terminal, whose screen a message shows (AI-46).
vi.mock("@/features/terminal/session", async (importActual) => {
  const rows = ["$ make", "error: no rule", "$ "];
  const line = (y: number) => ({ isWrapped: false, translateToString: () => rows[y] });
  const term = { rows: rows.length, buffer: { active: { type: "normal", length: rows.length, baseY: 0, getLine: (y: number) => (rows[y] === undefined ? undefined : line(y)) } } };
  return { ...(await importActual<typeof import("@/features/terminal/session")>()), getSession: (tabId: string) => (tabId === "tab-1" ? { term } : undefined) };
});

const { setSlotScreen, turnContext } = await import("./actions");
const { blankSlot, HOME_SLOT, useAi } = await import("./store");
const { useTabs } = await import("@/app/tabs");

const provider: AiProviderView = {
  id: "p",
  name: "p",
  protocol: "chat_completions",
  base_url: "https://example.com/v1",
  has_api_key: true,
  auth_header: "authorization",
  models: [{ id: "m", name: "m", context_window: null, max_output_tokens: null, efforts: null, adaptive_thinking: null }],
  updated_at: 0,
};
const view = (host_id: string | null, quick_target: QuickTarget | null): AiConversationView => ({
  id: "c1",
  title: "t",
  host_id,
  quick_target,
  origin: null,
  pinned: false,
  context_start: null,
  effort: null,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
});
const quickTab: SessionTab = { id: "tab-1", hostId: null, target: { address: "10.0.0.8", port: 22, username: "root" }, title: "root@10.0.0.8", status: "connected", sessionId: "s1" };

function attach(slotId: string, conversation: AiConversationView, mode: "manual" | "bypass" = "manual") {
  useAi.setState({ slots: { [slotId]: { ...blankSlot(mode), conversationId: conversation.id, conversation } } });
}
const settings = (send_terminal_screen: boolean): AiSettingsView => ({
  default_model: null,
  default_effort: null,
  search_provider_id: null,
  builtin_skill_enabled: true,
  custom_instructions: "",
  send_terminal_screen,
});

beforeEach(() => {
  useAi.setState({ catalog: { providers: [provider], settings: null, loaded: true } });
  useTabs.setState({ tabs: [quickTab] });
});

describe("turnContext (AI-09)", () => {
  it("names the conversation's quick-connect target on the home tab, with no terminal (HOST-12)", () => {
    const target = { address: "2001:db8::1", port: 2222, username: "root" };
    attach(HOME_SLOT, view(null, target));
    expect(turnContext(HOME_SLOT)).toMatchObject({ host_id: null, target, tab: false, session_id: null });
    // A user name typed at the prompt can have a space; the target goes as it is.
    const domainUser = { address: "10.0.0.7", port: 22, username: "domain user" };
    attach(HOME_SLOT, view(null, domainUser));
    expect(turnContext(HOME_SLOT)).toMatchObject({ host_id: null, target: domainUser });
  });

  it("names the conversation's host on the home tab", () => {
    attach(HOME_SLOT, view("h1", null));
    expect(turnContext(HOME_SLOT)).toMatchObject({ host_id: "h1", target: null, tab: false });
  });

  it("names the tab's target in a terminal tab, which the next message moves the conversation to", () => {
    attach(quickTab.id, view(null, { address: "10.0.0.7", port: 22, username: "root" }));
    expect(turnContext(quickTab.id)).toMatchObject({ host_id: null, target: quickTab.target, tab: true, session_id: "s1" });
  });

  it("names the conversation's permission mode (§13.5)", () => {
    attach(quickTab.id, view(null, null), "bypass");
    expect(turnContext(quickTab.id)?.permission_mode).toBe("bypass");
    attach(HOME_SLOT, view(null, null));
    expect(turnContext(HOME_SLOT)?.permission_mode).toBe("manual");
  });

  it("reads the connected tab's screen while Send Terminal Screen is on (AI-46)", () => {
    attach(quickTab.id, view(null, null));
    expect(turnContext(quickTab.id)?.screen).toEqual({ text: "$ make\nerror: no rule\n$", alternate: false });
    // Settings → AI gives the default, which the + menu changes for the conversation until the app quits.
    useAi.setState({ catalog: { providers: [provider], settings: settings(false), loaded: true } });
    expect(turnContext(quickTab.id)?.screen).toBeNull();
    setSlotScreen(quickTab.id, true);
    expect(turnContext(quickTab.id)?.screen).not.toBeNull();
    expect(useAi.getState().screens).toEqual({ c1: true });
    useAi.setState({ catalog: { providers: [provider], settings: settings(true), loaded: true } });
    setSlotScreen(quickTab.id, false);
    expect(turnContext(quickTab.id)?.screen).toBeNull();
    attach(quickTab.id, view(null, null));
    expect(turnContext(quickTab.id)?.screen).not.toBeNull();
    // Not while the tab is disconnected, nor on the home tab, which has no terminal.
    useTabs.setState({ tabs: [{ ...quickTab, status: "disconnected" }] });
    expect(turnContext(quickTab.id)?.screen).toBeNull();
    attach(HOME_SLOT, view(null, null));
    expect(turnContext(HOME_SLOT)?.screen).toBeNull();
  });
});
