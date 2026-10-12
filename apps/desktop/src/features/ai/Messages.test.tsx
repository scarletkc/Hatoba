import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AiAutoVerdict, AiEntryView, AiFinish, AiToolCall } from "@/ipc/types";
import type { Slot } from "./store";
import type { CallInfo } from "./turn";

// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { MessageList } = await import("./Messages");
const { blankSlot } = await import("./store");
const { newTurn } = await import("./turn");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function mount(slot: Slot): HTMLDivElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<MessageList slotId="home" slot={slot} host={null} empty={null} />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host;
}

function render(slot: Slot): string {
  return mount(slot).textContent ?? "";
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

describe("Auto mode on calls (AI-47)", () => {
  const restart: AiToolCall = { id: "call_1", name: "run_command", arguments: JSON.stringify({ command: "sudo systemctl restart nginx" }) };
  const calling: AiEntryView = { ...(reply("tool_calls") as Extract<AiEntryView, { role: "assistant" }>), tool_calls: [restart] };
  const verdict = (decision: AiAutoVerdict["decision"], concerns: string[] = [], failure: string | null = null): AiAutoVerdict => ({
    provider_id: "p",
    model_id: "judge",
    decision,
    answers: failure
      ? []
      : [
          { question: "requested", probability: 0.4 },
          { question: "changes_anything", probability: 0.71 },
          { question: "destroys_data", probability: 0.64 },
        ],
    concerns,
    failure,
  });
  const result = (auto: AiAutoVerdict | null): AiEntryView => ({ role: "tool", entry_id: "t1", created_at: 0, tool_call_id: "call_1", status: "ok", content: "done", auto });
  const waiting = (call: CallInfo): Slot => ({ ...slot([user, calling]), turn: { ...newTurn(), phase: "tools", call } });

  it("marks a call Auto ran with a chip whose title lists the answers, and says so in its block", () => {
    const host = mount(slot([user, calling, result(verdict("run"))]));
    const chip = [...host.querySelectorAll("span")].find((x) => x.textContent === "Auto");
    expect(chip?.title).toBe("requested 40% · changes something 71% · destroys data 64%");
    act(() => host.querySelector<HTMLButtonElement>("button[aria-expanded]")?.click());
    expect(host.textContent).toContain("Auto ran this call without asking.");
    expect(host.textContent).toContain("The judge’s answers: requested 40% · changes something 71% · destroys data 64%");
  });

  it("says in the block what concerned the judge when the card asked, and shows no chip", () => {
    const host = mount(slot([user, calling, result(verdict("ask", ["destroys_data", "requested"]))]));
    expect([...host.querySelectorAll("span")].some((x) => x.textContent === "Auto")).toBe(false);
    act(() => host.querySelector<HTMLButtonElement>("button[aria-expanded]")?.click());
    expect(host.textContent).toContain("Auto asked because: may delete or overwrite data (64%), not clearly asked for (40%)");
  });

  it("shows a call being checked", () => {
    expect(render(waiting({ id: "call_1", state: "judging" }))).toContain("Checking");
  });

  it("says on the approval card why Auto asks", () => {
    expect(render(waiting({ id: "call_1", state: "approval", auto: { verdict: verdict("ask", ["changes_anything", "requested"]) } }))).toContain(
      "Auto asked because: changes something (71%), not clearly asked for (40%)",
    );
    expect(render(waiting({ id: "call_1", state: "approval", auto: { rule: { kind: "command", command: "rm -r" } } }))).toContain(
      "Auto always asks when the input names rm -r.",
    );
    expect(render(waiting({ id: "call_1", state: "approval", auto: { rule: { kind: "after_reject" } } }))).toContain(
      "You rejected an earlier call in this reply, so Auto asks about the rest of it.",
    );
    const unavailable = render(waiting({ id: "call_1", state: "approval", auto: { verdict: verdict("ask", [], "unavailable") } }));
    expect(unavailable).toContain("Auto could not judge this call: no judge is available. Choose one in Settings → AI.");
    expect(unavailable).toContain("Open Settings → AI");
    expect(render(waiting({ id: "call_1", state: "approval", auto: { verdict: verdict("ask", [], "overloaded_v2") } }))).toContain("Auto could not judge this call.");
    expect(render(waiting({ id: "call_1", state: "approval" }))).not.toContain("Auto");
  });
});
