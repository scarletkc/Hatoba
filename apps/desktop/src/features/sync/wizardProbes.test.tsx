import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SyncConfigInput, SyncTestResult } from "@/ipc/types";
import { D1Step } from "./D1Step";
import { WorkerStep } from "./WorkerStep";
import { EMPTY_D1, EMPTY_WORKER, type D1Form, type WorkerForm } from "./wizardTypes";

// Each `sync_test` call waits until the test answers it, so answers can arrive out of order.
const calls: { config: SyncConfigInput; answer: (r: SyncTestResult) => void }[] = [];
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    sync_test: (config: SyncConfigInput) => new Promise<SyncTestResult>((answer) => calls.push({ config, answer })),
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const result = (initialized: boolean, databases: SyncTestResult["databases"] = null): SyncTestResult => ({
  ok: true,
  initialized,
  version: null,
  latency_ms: 10,
  databases,
  error: null,
});

let root: Root | null = null;
let host: HTMLElement | null = null;
beforeEach(() => {
  calls.length = 0;
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
});
afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
});

/** Types into an input as a user would, so React sees the change. */
function type(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const button = (label: RegExp) => [...host!.querySelectorAll("button")].find((b) => label.test(b.textContent ?? ""))!;
const answer = async (i: number, r: SyncTestResult) => act(async () => calls[i].answer(r));

describe("Worker step", () => {
  // The wizard keeps the form while the step comes and goes, as `SyncWizard` does.
  let form: WorkerForm = EMPTY_WORKER;
  function Wizard({ shown }: { shown: boolean }) {
    const [state, setState] = useState(EMPTY_WORKER);
    form = state;
    if (!shown) return null;
    return <WorkerStep form={state} update={(p) => setState((f) => ({ ...f, ...p }))} onBack={() => {}} onNext={() => {}} />;
  }

  it("drops a test answer that arrives after the step was left and tested again", async () => {
    act(() => root!.render(<Wizard shown />));
    type(host!.querySelector<HTMLInputElement>("input[inputmode=url]")!, "https://a.example.workers.dev");
    act(() => button(/测试连接|Test Connection/).click());

    act(() => root!.render(<Wizard shown={false} />));
    act(() => root!.render(<Wizard shown />));
    type(host!.querySelector<HTMLInputElement>("input[inputmode=url]")!, "https://b.example.workers.dev");
    act(() => button(/测试连接|Test Connection/).click());
    expect(calls).toHaveLength(2);

    await answer(1, result(false));
    await answer(0, result(true));
    expect(form.url).toBe("https://b.example.workers.dev");
    expect(form.test.status === "ok" && form.test.result.initialized).toBe(false);
  });

  it("lets a test that was running when the step was left run again on return", async () => {
    act(() => root!.render(<Wizard shown />));
    type(host!.querySelector<HTMLInputElement>("input[inputmode=url]")!, "https://a.example.workers.dev");
    act(() => button(/测试连接|Test Connection/).click());
    act(() => root!.render(<Wizard shown={false} />));
    act(() => root!.render(<Wizard shown />));

    expect(form.test.status).toBe("idle");
    expect(button(/测试连接|Test Connection/).disabled).toBe(false);
    act(() => button(/测试连接|Test Connection/).click());
    await answer(0, result(false));
    expect(form.test.status).toBe("testing");
    await answer(1, result(true));
    expect(form.test.status === "ok" && form.test.result.initialized).toBe(true);
  });

  it("drops a test answer for a URL that was edited meanwhile", async () => {
    act(() => root!.render(<Wizard shown />));
    const url = host!.querySelector<HTMLInputElement>("input[inputmode=url]")!;
    type(url, "https://a.example.workers.dev");
    act(() => button(/测试连接|Test Connection/).click());
    type(url, "https://b.example.workers.dev");

    await answer(0, result(true));
    expect(form.test.status).toBe("idle");
  });
});

describe("D1 step", () => {
  let form: D1Form = EMPTY_D1;
  function Wizard({ shown = true }: { shown?: boolean }) {
    const [state, setState] = useState(EMPTY_D1);
    form = state;
    if (!shown) return null;
    return <D1Step form={state} update={(p) => setState((f) => ({ ...f, ...p }))} onBack={() => {}} onNext={() => {}} />;
  }
  const db = (id: string) => ({ id, name: id, region: null });

  it("drops a token check that was overtaken by an edit and a new check", async () => {
    act(() => root!.render(<Wizard />));
    const [account, token] = [...host!.querySelectorAll<HTMLInputElement>("input")];
    type(account, "account-a");
    type(token, "token-a");
    act(() => button(/验证 Token|Verify Token/).click());
    type(token, "token-b");
    act(() => button(/验证 Token|Verify Token/).click());
    expect(calls.map((c) => c.config.kind === "d1" && c.config.api_token)).toEqual(["token-a", "token-b"]);

    await answer(1, result(false, [db("b1"), db("b2")]));
    await answer(0, result(false, [db("a1")]));
    expect(form.databases.map((d) => d.id)).toEqual(["b1", "b2"]);
    expect(form.databaseId).toBe("");
    expect(calls).toHaveLength(2);
  });

  it("lets a token check that was running when the step was left run again on return", async () => {
    act(() => root!.render(<Wizard />));
    const [account, token] = [...host!.querySelectorAll<HTMLInputElement>("input")];
    type(account, "account-a");
    type(token, "token-a");
    act(() => button(/验证 Token|Verify Token/).click());
    act(() => root!.render(<Wizard shown={false} />));
    act(() => root!.render(<Wizard />));

    expect(form.verify.status).toBe("idle");
    expect(button(/验证 Token|Verify Token/).disabled).toBe(false);
    await answer(0, result(false, [db("a1")]));
    expect(form.verify.status).toBe("idle");
  });

  it("checks the chosen database again when its check was running as the step was left", async () => {
    act(() => root!.render(<Wizard />));
    const [account, token] = [...host!.querySelectorAll<HTMLInputElement>("input")];
    type(account, "account-a");
    type(token, "token-a");
    act(() => button(/验证 Token|Verify Token/).click());
    // A single database is picked, and checked, right away.
    await answer(0, result(false, [db("a1")]));
    expect(form.checking).toBe(true);
    act(() => root!.render(<Wizard shown={false} />));
    act(() => root!.render(<Wizard />));

    expect(calls).toHaveLength(3);
    expect(calls[2].config.kind === "d1" && calls[2].config.database_id).toBe("a1");
    await answer(1, result(false));
    expect(form.checking).toBe(true);
    await answer(2, result(true));
    expect(form.checking).toBe(false);
    expect(form.initialized).toBe(true);
  });
});
