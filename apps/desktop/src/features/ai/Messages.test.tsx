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
const { noteBlock } = await import("./attachments");

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

describe("permission mode dividers (AI-16)", () => {
  const sent = (id: string, mode: "manual" | "auto" | "bypass" | null, text: string): AiEntryView => ({
    role: "user",
    entry_id: id,
    created_at: 0,
    text: mode ? `${noteBlock({ kind: "permission_mode", mode })}

${text}` : text,
  });
  const answer = (id: string): AiEntryView => ({ ...(reply("stop") as Extract<AiEntryView, { role: "assistant" }>), entry_id: id, text: "Done." });

  it("shows none above the first message, whose note only names the mode it started in", () => {
    const text = render(slot([sent("u1", "bypass", "first"), answer("a1")]));
    expect(text).toContain("first");
    expect(text).not.toContain("Permission mode");
  });

  it("shows a switch on a later message", () => {
    expect(render(slot([sent("u1", "bypass", "first"), answer("a1"), sent("u2", "manual", "second")]))).toContain("Permission mode: Manual approval");
    expect(render(slot([sent("u1", null, "first"), answer("a1"), sent("u2", "auto", "second")]))).toContain("Permission mode: Auto");
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
  const waiting = (call: CallInfo): Slot => ({ ...slot([user, calling]), turn: { ...newTurn(), phase: "tools", calls: [call] } });

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
    expect(render(waiting({ id: "call_1", state: "approval", auto: { rule: { kind: "editing_submission" } } }))).toContain(
      "Auto always asks when the input edits the terminal’s line and then submits it.",
    );
    const unavailable = render(waiting({ id: "call_1", state: "approval", auto: { verdict: verdict("ask", [], "unavailable") } }));
    expect(unavailable).toContain("Auto could not judge this call: no judge is available. Choose one in Settings → AI.");
    expect(unavailable).toContain("Open Settings → AI");
    expect(render(waiting({ id: "call_1", state: "approval", auto: { verdict: verdict("ask", [], "overloaded_v2") } }))).toContain("Auto could not judge this call.");
    expect(render(waiting({ id: "call_1", state: "approval" }))).not.toContain("Auto");
  });
});

describe("a command's folder on the approval card (AI-17)", () => {
  const asking = (args: Record<string, unknown>): Slot => {
    const call: AiToolCall = { id: "call_1", name: "run_command", arguments: JSON.stringify(args) };
    const calling: AiEntryView = { ...(reply("tool_calls") as Extract<AiEntryView, { role: "assistant" }>), tool_calls: [call] };
    return { ...slot([user, calling]), turn: { ...newTurn(), phase: "tools", calls: [{ id: "call_1", state: "approval" }] } };
  };

  it("shows the whole path, with hidden characters as escapes, and nothing without one", () => {
    expect(render(asking({ command: "npm test", workdir: "/srv/app" }))).toContain("Folder/srv/app");
    const hidden = render(asking({ command: "npm test", workdir: "/srv/‮ppa" }));
    expect(hidden).toContain("/srv/\\u{202e}ppa");
    expect(hidden).toContain("Hidden characters shown as escapes");
    expect(render(asking({ command: "npm test" }))).not.toContain("Folder");
  });

  it("lets Edit change it, with the home folder for an empty one", () => {
    const host = mount(asking({ command: "npm test", workdir: "/srv/app" }));
    act(() => [...host.querySelectorAll("button")].find((b) => b.textContent === "Edit")?.click());
    const input = host.querySelector<HTMLInputElement>("input[placeholder='Home folder']");
    expect(input?.value).toBe("/srv/app");
  });
});

describe("terminal key approval (AI-13)", () => {
  const waiting = (args: Record<string, unknown>): Slot => ({
    ...slot([user, { ...(reply("tool_calls") as Extract<AiEntryView, { role: "assistant" }>), tool_calls: [{ id: "keys_1", name: "send_input", arguments: JSON.stringify(args) }] }]),
    turn: { ...newTurn(), phase: "tools", calls: [{ id: "keys_1", state: "approval" }] },
  });
  const click = (host: HTMLElement, text: string) => act(() => [...host.querySelectorAll("button")].find((b) => b.textContent?.trim() === text)?.click());
  const input = (element: HTMLInputElement | HTMLTextAreaElement, value: string) => {
    const prototype = element instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    act(() => {
      Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(element, value);
      element.dispatchEvent(new Event("input", { bubbles: true }));
    });
  };
  const shown = (host: HTMLElement) => [...host.querySelectorAll("kbd")].map((e) => e.textContent);
  const chips = (host: HTMLElement) => [...host.querySelectorAll("[data-key]")].map((e) => e.textContent);
  const field = (host: HTMLElement) => host.querySelector<HTMLInputElement>("input[list]")!;
  const press = (host: HTMLElement, key: string) => act(() => {
    field(host).dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });
  const type = (host: HTMLElement, text: string) => {
    input(field(host), text);
    press(host, "Enter");
  };
  const remove = (host: HTMLElement, label: string) => act(() => {
    const button = host.querySelector<HTMLButtonElement>(`button[aria-label="Remove key ${label}"]`);
    expect(button, label).not.toBeNull();
    button!.click();
  });
  /** Runs the card and returns the arguments it ran with, or null when Run did nothing. */
  const runWith = async (host: HTMLElement) => {
    const actions = await import("./actions");
    const decide = vi.spyOn(actions, "decideCall").mockImplementation(() => {});
    try {
      click(host, "Run");
      if (!decide.mock.calls.length) return null;
      const decision = decide.mock.calls[0][1];
      if (decision.kind !== "run") throw new Error("expected Run");
      return JSON.parse(decision.edited!) as Record<string, unknown>;
    } finally {
      decide.mockRestore();
    }
  };

  it("shows the controls in the text and every key, and keeps the text's bytes when only a key changes", async () => {
    const text = "\x18\x1b\\x18^X";
    const host = mount(waiting({ text, key: "ctrl_c", keys: ["ctrl+x", "n"] }));
    expect(host.textContent).toContain("⟦^X⟧⟦Esc⟧");
    expect(shown(host)).toEqual(["Ctrl+C", "Ctrl+X", "n"]);
    expect(host.textContent).toContain("Then press Ctrl+C Ctrl+X n · Waits up to 10 s for output");
    click(host, "Edit");
    expect(host.querySelector("textarea")?.value).toBe("⟦^X⟧⟦Esc⟧\\x18^X");
    expect(host.textContent).toContain("Control characters show as ⟦^X⟧, ⟦Esc⟧ and so on.");
    expect(chips(host)).toEqual(["Ctrl+C", "Ctrl+X", "n"]);
    remove(host, "Ctrl+X");
    type(host, "ctrl+o");
    expect(await runWith(host)).toEqual({ text, keys: ["ctrl_c", "ctrl+o", "n"] });
  });

  it.each(["cd C:\\temp", "grep '^Error' log"])("sends %s exactly as typed in the editor", async (text) => {
    const host = mount(waiting({ text: "ls", key: "enter" }));
    click(host, "Edit");
    input(host.querySelector("textarea")!, text);
    expect((await runWith(host))?.text).toBe(text);
  });

  it("names a space Space, in the view and in Edit", () => {
    const host = mount(waiting({ keys: [" ", "space", "alt+space"] }));
    expect(shown(host)).toEqual(["Space", "Space", "Alt+Space"]);
    click(host, "Edit");
    expect(chips(host)).toEqual(["Space", "Space", "Alt+Space"]);
  });

  it("shows Right as a key of its own, with nothing between the keys that reads as one", () => {
    const host = mount(waiting({ keys: ["up", "right", "enter"] }));
    expect(shown(host)).toEqual(["↑", "→", "Enter"]);
    expect(host.textContent?.match(/→/g)).toHaveLength(1);
  });

  it("names keys as keyboards print them", () => {
    const host = mount(waiting({ keys: ["ctrl+x", "alt+f", "alt+F", "shift+tab", "f10", "page_down", "ctrl+["] }));
    expect(shown(host)).toEqual(["Ctrl+X", "Alt+f", "Alt+F", "Shift+Tab", "F10", "Page Down", "Ctrl+["]);
  });

  it("adds keys where the caret is and removes any key, keeping focus in the field", async () => {
    const host = mount(waiting({ keys: ["ctrl+x", "n"] }));
    click(host, "Edit");
    remove(host, "Ctrl+X");
    expect(document.activeElement).toBe(field(host));
    type(host, "Esc");
    type(host, " ");
    expect(chips(host)).toEqual(["Esc", "Space", "n"]);
    expect(document.activeElement).toBe(field(host));
    press(host, "End");
    type(host, "enter");
    press(host, "Home");
    type(host, "tab");
    expect(chips(host)).toEqual(["Tab", "Esc", "Space", "n", "Enter"]);
    // ← and → move the caret over the keys, Backspace and Delete remove the key on either side.
    press(host, "ArrowRight");
    press(host, "Delete");
    press(host, "ArrowLeft");
    press(host, "Backspace");
    press(host, "ArrowRight");
    press(host, "ArrowRight");
    press(host, "Backspace");
    expect(chips(host)).toEqual(["Esc", "Enter"]);
    expect(await runWith(host)).toEqual({ text: "", keys: ["esc", "enter"] });
  });

  it("adds what was typed when the field loses focus, and Esc drops it without leaving Edit", async () => {
    const host = mount(waiting({ text: "ls", keys: ["enter"] }));
    click(host, "Edit");
    act(() => field(host).focus());
    input(field(host), "q");
    press(host, "Escape");
    expect(field(host).value).toBe("");
    expect(host.querySelector("textarea")).not.toBeNull();
    expect(document.activeElement).toBe(field(host));
    press(host, "Home");
    input(field(host), "ctrl+u");
    act(() => field(host).blur());
    expect(chips(host)).toEqual(["Ctrl+U", "Enter"]);
    expect(await runWith(host)).toEqual({ text: "ls", keys: ["ctrl+u", "enter"] });
  });

  it("marks a key Hatoba can't press on its own chip, and Run waits until it is removed", async () => {
    const host = mount(waiting({ keys: ["ctrl+x", "pagedown"] }));
    click(host, "Edit");
    const bad = () => [...host.querySelectorAll("[data-key]")].filter((e) => e.className.includes("keyChipBad")).map((e) => e.textContent);
    expect(bad()).toEqual(["pagedown"]);
    expect(host.textContent).toContain("Keys must be single characters or names such as enter, ctrl+x, alt+f or page_down.");
    type(host, "ctrl+");
    expect(bad()).toEqual(["pagedown", "ctrl+"]);
    expect(await runWith(host)).toBeNull();
    remove(host, "pagedown");
    remove(host, "ctrl+");
    expect(host.textContent).not.toContain("Keys must be");
    expect(await runWith(host)).toEqual({ text: "", keys: ["ctrl+x"] });
  });

  it("shows the control hint only while the edited text has a marker", () => {
    const host = mount(waiting({ text: "ls", keys: ["enter"] }));
    click(host, "Edit");
    expect(host.textContent).not.toContain("Control characters show");
    input(host.querySelector("textarea")!, "⟦Esc⟧");
    expect(host.textContent).toContain("Control characters show");
    input(host.querySelector("textarea")!, "ls");
    expect(host.textContent).not.toContain("Control characters show");
  });

  it("does not explain literal marker text as a control", () => {
    const host = mount(waiting({ text: "literal ⟦Esc⟧", keys: ["enter"] }));
    click(host, "Edit");
    expect(host.textContent).not.toContain("Control characters show");
  });
});

describe("calls that run together (§13.1)", () => {
  const read = (id: string, path: string): AiToolCall => ({ id, name: "read_file", arguments: JSON.stringify({ path }) });
  const calls = [read("call_1", "/etc/hosts"), read("call_2", "/etc/hostname"), read("call_3", "/etc/motd")];
  const calling: AiEntryView = { ...(reply("tool_calls") as Extract<AiEntryView, { role: "assistant" }>), tool_calls: calls };
  const done: AiEntryView = { role: "tool", entry_id: "t1", created_at: 0, tool_call_id: "call_2", status: "ok", content: "prod-api", auto: null };

  it("shows each running call as running, a finished one with its result, and the rest as queued", () => {
    const running: CallInfo[] = [
      { id: "call_1", state: "running" },
      { id: "call_2", state: "running" },
    ];
    const host = mount({ ...slot([user, calling, done]), turn: { ...newTurn(), phase: "tools", calls: running } });
    const chips = [...host.querySelectorAll("[data-reveal^=\"call:\"]")].map((block) => block.textContent ?? "");
    expect(chips).toHaveLength(3);
    expect(chips[0]).toContain("Running");
    expect(chips[1]).not.toContain("Running");
    expect(chips[2]).toContain("Queued");
  });
});
