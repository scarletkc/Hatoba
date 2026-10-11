import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { setOverlaysLocked } from "@/components/overlay";
import type { AiFileChange, AiFileRevert } from "@/ipc/types";
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

const change = (callId: string, path: string, created = false): AiFileChange => ({
  response_id: "a1",
  tool_call_id: callId,
  server: "root@10.0.0.5:22",
  path,
  created,
  state: "written",
});
const changes = [change("c1", "/etc/nginx/nginx.conf"), change("c2", "/etc/nginx/conf.d/api.conf", true)];

const button = (label: string) => [...document.body.querySelectorAll("button")].find((b) => b.textContent === label)!;
const click = async (label: string) => {
  await act(async () => button(label).click());
};

describe("the dialog of an edit that deletes file changes (AI-26, AI-41)", () => {
  it("lists the files newest call first with their server, and has no default action", () => {
    render();
    act(() => void askEditedFiles(2, changes, async () => ({ status: "reverted", wrote: true })));
    const items = [...document.body.querySelectorAll("li")].map((li) => li.textContent);
    expect(items[0]).toContain("/etc/nginx/conf.d/api.conf");
    expect(items[0]).toContain("root@10.0.0.5:22");
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
      answer = askEditedFiles(2, changes, async (c) => {
        order.push(c.tool_call_id);
        return { status: "reverted", wrote: true };
      });
    });
    await click("Revert and Send");
    await expect(answer).resolves.toBe(true);
    expect(order).toEqual(["c2", "c1"]);
    expect(document.body.querySelector("li")).toBeNull();
  });

  it("lists why a change was not reverted, then sends anyway or cancels", async () => {
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
        answer = askEditedFiles(2, changes, async (c) => outcomes[c.tool_call_id]!);
      });
      await click("Revert and Send");
      const text = document.body.textContent ?? "";
      expect(text).toContain("Connect it to admin@10.0.0.9:22");
      expect(text).toContain("The file no longer holds what the call wrote.");
      expect(button("Keep Files and Send")).toBeUndefined();
      await click(choice);
      await expect(answer).resolves.toBe(sent);
      act(() => unmount?.());
      unmount = null;
    }
  });

  it("lists a revert that threw as failed instead of hanging", async () => {
    render();
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = askEditedFiles(2, changes, async () => {
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
      answer = askEditedFiles(2, changes, async () => {
        reverts++;
        return { status: "reverted", wrote: true };
      });
    });
    await click("Keep Files and Send");
    await expect(answer).resolves.toBe(true);
    act(() => {
      answer = askEditedFiles(2, changes, async () => ({ status: "reverted", wrote: true }));
    });
    await click("Cancel");
    await expect(answer).resolves.toBe(false);
    expect(reverts).toBe(0);
  });

  it("is cancelled when the vault locks", async () => {
    render();
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = askEditedFiles(2, changes, async () => ({ status: "reverted", wrote: true }));
    });
    act(() => setOverlaysLocked(true));
    await expect(answer).resolves.toBe(false);
    act(() => setOverlaysLocked(false));
    expect(document.body.querySelector("li")).toBeNull();
  });
});
