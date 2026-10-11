import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type { AiFilePreview } from "@/ipc/types";
import { FileChange } from "./FileChange";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let host: HTMLDivElement | null = null;
let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(preview: AiFilePreview) {
  host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<FileChange state={{ preview, loading: false, failure: null }} />));
  unmount = () => {
    root.unmount();
    host?.remove();
  };
  return host;
}

describe("the change on the approval card (AI-39, AI-40)", () => {
  it("renders a long new file a page at a time, and every line on request", () => {
    const after = Array.from({ length: 1000 }, (_, i) => `line ${i + 1}`).join("\n") + "\n";
    const el = render({ path: "/srv/big.txt", before: null, after, error: null, crlf: false });
    const text = () => el.textContent ?? "";
    expect(text()).toContain("line 400");
    expect(text()).not.toContain("line 401");
    const more = () => [...el.querySelectorAll("button")].find((b) => b.textContent?.includes("600"));
    expect(more()).toBeDefined();
    act(() => more()!.click());
    expect(text()).toContain("line 800");
    expect(text()).not.toContain("line 801");
    const rest = [...el.querySelectorAll("button")].find((b) => b.textContent?.includes("200"));
    act(() => rest!.click());
    expect(text()).toContain("line 1000");
    expect(el.querySelectorAll("button")).toHaveLength(0);
  });

  it("shows why a call cannot apply instead of a diff", () => {
    const el = render({ path: "/etc/hosts", before: null, after: null, error: "old_string was not found in /etc/hosts", crlf: false });
    expect(el.querySelector('[role="alert"]')?.textContent).toContain("old_string was not found");
    expect(el.querySelector('[role="group"]')).toBeNull();
  });
});
