import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiEntryView, AiFinish } from "@/ipc/types";
import type { Slot } from "./store";

// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { MessageList } = await import("./Messages");
const { blankSlot } = await import("./store");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(slot: Slot): string {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<MessageList slotId="home" slot={slot} host={null} empty={null} />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host.textContent ?? "";
}

const user: AiEntryView = { role: "user", entry_id: "u1", created_at: 0, text: "question" };
const reply = (finish: AiFinish, reasoning: string | null = null): AiEntryView => ({
  role: "assistant",
  entry_id: "a1",
  created_at: 0,
  provider_id: "p",
  model_id: "m",
  text: "",
  reasoning,
  tool_calls: [],
  finish,
  usage: null,
});
const slot = (entries: AiEntryView[], outcome: Slot["outcome"] = null): Slot => ({ ...blankSlot(), conversationId: "c1", entries, outcome });

describe("a reply without an answer (§13.1)", () => {
  it("says so under a reply of reasoning only", () => {
    expect(render(slot([user, reply("stop", "Thinking it over.")]))).toContain("The model gave no answer.");
  });

  it("says so when the provider reported tool calls and sent none", () => {
    expect(render(slot([user, reply("tool_calls")]))).toContain("The model gave no answer.");
  });

  it("leaves it to the outcome while the turn's outcome says it", () => {
    const text = render(slot([user, reply("tool_calls")], { reason: "empty" }));
    expect(text).toContain("The model ended its reply without an answer.");
    expect(text).not.toContain("The model gave no answer.");
  });
});
