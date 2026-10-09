import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { confirm, ConfirmHost, Modal, setOverlaysLocked } from "./overlay";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  setOverlaysLocked(false);
});

function render(ui: React.ReactNode) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

describe("Modal", () => {
  it("focuses the first control in the tab order", () => {
    render(
      <Modal>
        <button type="button" disabled>
          Disabled
        </button>
        <button type="button">First</button>
        <button type="button">Second</button>
      </Modal>,
    );
    expect(document.activeElement?.textContent).toBe("First");
  });

  it("skips the unselected tabs of a tablist", () => {
    render(
      <Modal>
        <div role="tablist">
          <button type="button" role="tab" tabIndex={-1}>
            General
          </button>
          <button type="button" role="tab" tabIndex={0} aria-selected>
            About
          </button>
        </div>
        <button type="button">Close</button>
      </Modal>,
    );
    expect(document.activeElement?.textContent).toBe("About");
  });
});

describe("vault lock", () => {
  it("hides an open modal while locked and shows it again after unlocking", () => {
    render(
      <Modal>
        <textarea defaultValue="SECRET-KEY-MARKER" />
      </Modal>,
    );
    expect(document.body.textContent).toContain("SECRET-KEY-MARKER");
    act(() => setOverlaysLocked(true));
    expect(document.body.textContent).not.toContain("SECRET-KEY-MARKER");
    act(() => setOverlaysLocked(false));
    expect(document.body.querySelector("textarea")?.value).toBe("SECRET-KEY-MARKER");
  });

  it("cancels a pending confirmation when the vault locks", async () => {
    render(<ConfirmHost />);
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = confirm({ title: "Paste 2 lines?", confirmLabel: "Paste" });
    });
    expect(document.body.textContent).toContain("Paste 2 lines?");
    act(() => setOverlaysLocked(true));
    await expect(answer).resolves.toBe(false);
    expect(document.body.textContent).not.toContain("Paste 2 lines?");
  });

  it("refuses new confirmations while locked", async () => {
    render(<ConfirmHost />);
    act(() => setOverlaysLocked(true));
    let answer: Promise<boolean> | undefined;
    act(() => {
      answer = confirm({ title: "Delete?", confirmLabel: "Delete" });
    });
    await expect(answer).resolves.toBe(false);
    expect(document.body.textContent).not.toContain("Delete?");
  });
});
