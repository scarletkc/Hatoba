import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionTab } from "@/app/tabs";
import type { AiConversationView, AiProviderView } from "@/ipc/types";

// turnContext never calls the backend; this keeps the real IPC layer out of the test.
vi.mock("@/ipc/api", async (importActual) => ({ ...(await importActual<typeof import("@/ipc/api")>()), api: {} }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { turnContext } = await import("./actions");
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
const view = (host_id: string | null, quick_target: string | null): AiConversationView => ({
  id: "c1",
  title: "t",
  host_id,
  quick_target,
  pinned: false,
  context_start: null,
  effort: null,
  created_at: 0,
  updated_at: 0,
  last_activity: 0,
});
const quickTab: SessionTab = { id: "tab-1", hostId: null, target: { address: "10.0.0.8", port: 22, username: "root" }, title: "root@10.0.0.8", status: "connected", sessionId: "s1" };

function attach(slotId: string, conversation: AiConversationView) {
  useAi.setState({ slots: { [slotId]: { ...blankSlot("manual"), conversationId: conversation.id, conversation } } });
}

beforeEach(() => {
  useAi.setState({ catalog: { providers: [provider], settings: null, loaded: true } });
  useTabs.setState({ tabs: [quickTab] });
});

describe("turnContext (AI-09)", () => {
  it("names the conversation's quick-connect target on the home tab, with no terminal (HOST-12)", () => {
    attach(HOME_SLOT, view(null, "root@[2001:db8::1]:2222"));
    expect(turnContext(HOME_SLOT)).toMatchObject({ host_id: null, target: { address: "2001:db8::1", port: 2222, username: "root" }, tab: false, session_id: null });
  });

  it("names the conversation's host on the home tab", () => {
    attach(HOME_SLOT, view("h1", null));
    expect(turnContext(HOME_SLOT)).toMatchObject({ host_id: "h1", target: null, tab: false });
  });

  it("names the tab's target in a terminal tab, which the next message moves the conversation to", () => {
    attach(quickTab.id, view(null, "root@10.0.0.7:22"));
    expect(turnContext(quickTab.id)).toMatchObject({ host_id: null, target: quickTab.target, tab: true, session_id: "s1" });
  });
});
