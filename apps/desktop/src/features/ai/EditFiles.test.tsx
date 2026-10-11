import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { setOverlaysLocked } from "@/components/overlay";
import type { AiFileChange, AiFilePreview, AiFileRevert } from "@/ipc/types";
import { askEditedFiles, EditFilesHost } from "./EditFiles";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  setOverlaysLocked(false);
});

function render() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<EditFilesHost />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

const SERVER = "root@10.0.0.5:22";
const change = (callId: string, path: string, created = false, server = SERVER): AiFileChange => ({
  response_id: "a1",
  tool_call_id: callId,
  server,
  path,
  created,
  state: "written",
});
const changes = [change("c1", "/etc/nginx/nginx.conf"), change("c2", "/etc/nginx/conf.d/api.conf", true)];
const reverted = async (): Promise<AiFileRevert> => ({ status: "reverted", wrote: true });

/** The dialog for `list`, with the tab connected to `server`. */
const ask = (
  list: AiFileChange[],
  revert: (change: AiFileChange) => Promise<AiFileRevert>,
  options: { server?: string | null; diff?: (change: AiFileChange) => Promise<AiFilePreview | null> } = {},
) => askEditedFiles({ later: 2, changes: list, server: options.server === undefined ? SERVER : options.server, revert, diff: options.diff });

const button = (label: string) => [...document.body.querySelectorAll("button")].find((b) => b.textContent === label)!;
const click = async (label: string) => {
  await act(async () => button(label).click());
};

describe("the dialog of an edit that deletes file changes (AI-26, AI-41)", () => {
  it("lists the files newest call first with their server, and has no default action", () => {
    render();
    act(() => void ask(changes, reverted));
    const items = [...document.body.querySelectorAll("li")].map((li) => li.textContent);
    expect(items[0]).toContain("/etc/nginx/conf.d/api.conf");
    expect(items[0]).toContain(SERVER);
    expect(items[0]).toContain("Revert deletes it");
    expect(items[1]).toContain("/etc/nginx/nginx.conf");
    expect(items[1]).not.toContain("Revert deletes it");
    expect(document.activeElement?.textContent).not.toMatch(/Send/);
  });

  it("reverts newest first and sends when every change was reverted", async () => {
    render();
    const order: string[] = [];
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = ask(changes, async (c) => {
        order.push(c.tool_call_id);
        return { status: "reverted", wrote: true };
      });
    });
    await click("Revert and Send");
    await expect(answer).resolves.toBe(true);
    expect(order).toEqual(["c2", "c1"]);
    expect(document.body.querySelector("li")).toBeNull();
  });

  it("lists why a change was not reverted, then sends anyway, which deletes its copy, or cancels", async () => {
    for (const [choice, sent] of [
      ["Send Anyway", true],
      ["Cancel", false],
    ] as const) {
      render();
      const outcomes: Record<string, AiFileRevert> = {
        c2: { status: "failed", reason: "other_server", detail: "admin@10.0.0.9:22" },
        c1: { status: "changed", path: "/etc/nginx/nginx.conf", current: "b\n", before: "a\n", later: false },
      };
      let answer: Promise<boolean> | undefined;
      act(() => {
        answer = ask(changes, async (c) => outcomes[c.tool_call_id]!);
      });
      await click("Revert and Send");
      const text = document.body.textContent ?? "";
      expect(text).toContain("Connect it to admin@10.0.0.9:22");
      expect(text).toContain("The file no longer holds what the call wrote.");
      expect(text).toContain("Send Anyway deletes this device’s copies of those changes, so they can no longer be reverted; Cancel keeps them.");
      expect(button("Keep Files and Send")).toBeUndefined();
      await click(choice);
      await expect(answer).resolves.toBe(sent);
      act(() => unmount?.());
      unmount = null;
    }
  });

  it("offers no revert while the tab is disconnected, so Cancel keeps the changes revertible", async () => {
    render();
    let reverts = 0;
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = ask(
        changes,
        async () => {
          reverts++;
          return { status: "reverted", wrote: true };
        },
        { server: null },
      );
    });
    expect(button("Revert and Send").disabled).toBe(true);
    expect(document.body.textContent).toContain("The tab is disconnected, so none of these files can be reverted now.");
    expect(document.body.textContent).toContain("reconnect the tab and send again");
    expect(button("Keep Files and Send").disabled).toBe(false);
    await click("Cancel");
    await expect(answer).resolves.toBe(false);
    expect(reverts).toBe(0);
  });

  it("reverts only the files on the tab's server, and lists the others as not reverted", async () => {
    render();
    const other = "admin@10.0.0.9:22";
    const mixed = [change("c1", "/etc/nginx/nginx.conf"), change("c2", "/etc/hosts", false, other)];
    const order: string[] = [];
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = ask(mixed, async (c) => {
        order.push(c.tool_call_id);
        return { status: "reverted", wrote: true };
      });
    });
    const elsewhere = `This tab is connected to ${SERVER}, so it cannot revert this file.`;
    const items = () => [...document.body.querySelectorAll("li")].map((li) => li.textContent ?? "");
    expect(items()[0]).toContain(elsewhere);
    expect(items()[1]).not.toContain(elsewhere);
    expect(button("Revert and Send").disabled).toBe(false);
    await click("Revert and Send");
    expect(order).toEqual(["c1"]);
    expect(items()[0]).toContain(elsewhere);
    expect(button("Send Anyway")).toBeDefined();
    await click("Cancel");
    await expect(answer).resolves.toBe(false);
    act(() => unmount?.());

    // None of them on its server: nothing to revert from this tab.
    render();
    act(() => void ask([change("c2", "/etc/hosts", false, other)], reverted));
    expect(button("Revert and Send").disabled).toBe(true);
    expect(document.body.textContent).toContain(`This tab is connected to ${SERVER}, so it cannot revert any of these files.`);
  });

  it("lists each file once, with how many changes it has and the lines a revert adds and removes", async () => {
    render();
    const twice = [change("c1", "/etc/nginx/nginx.conf"), change("c2", "/etc/hosts"), change("c3", "/etc/nginx/nginx.conf")];
    const order: string[] = [];
    act(() => {
      void ask(
        twice,
        async (c) => {
          order.push(c.tool_call_id);
          return { status: "reverted", wrote: true };
        },
        { diff: async (c) => ({ path: c.path, before: "a\n", after: c.tool_call_id === "c3" ? "a\nb\nc\n" : "b\n", error: null, crlf: false }) },
      );
    });
    await act(async () => {});
    const items = [...document.body.querySelectorAll("li")].map((li) => li.textContent ?? "");
    expect(items).toHaveLength(2);
    expect(items[0]).toContain("/etc/nginx/nginx.conf");
    expect(items[0]).toContain("2 changes");
    // c3 reverts to a: two lines removed; c1 reverts b to a: one added, one removed.
    expect(items[0]).toContain("+1");
    expect(items[0]).toContain("−3");
    expect(items[1]).not.toContain("changes");
    await click("Revert and Send");
    expect(order).toEqual(["c3", "c2", "c1"]);
  });

  it("lists a revert that threw as failed instead of hanging", async () => {
    render();
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = ask(changes, async () => {
        throw new Error("the vault is locked");
      });
    });
    await click("Revert and Send");
    expect(document.body.textContent).toContain("Revert failed.");
    await click("Cancel");
    await expect(answer).resolves.toBe(false);
  });

  it("keeps the files as they are, or cancels, without reverting", async () => {
    render();
    let reverts = 0;
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = ask(changes, async () => {
        reverts++;
        return { status: "reverted", wrote: true };
      });
    });
    await click("Keep Files and Send");
    await expect(answer).resolves.toBe(true);
    act(() => {
      answer = ask(changes, reverted);
    });
    await click("Cancel");
    await expect(answer).resolves.toBe(false);
    expect(reverts).toBe(0);
  });

  it("is cancelled when the vault locks", async () => {
    render();
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = ask(changes, reverted);
    });
    act(() => setOverlaysLocked(true));
    await expect(answer).resolves.toBe(false);
    act(() => setOverlaysLocked(false));
    expect(document.body.querySelector("li")).toBeNull();
  });
});
