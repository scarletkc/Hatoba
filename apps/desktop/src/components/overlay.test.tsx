import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Modal } from "./overlay";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
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
