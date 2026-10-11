import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

const startResizeDragging = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ startResizeDragging }),
}));

import { ResizeHandles } from "./ResizeHandles";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  startResizeDragging.mockClear();
});

function render(ui: React.ReactNode) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
  act(() => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

const handles = () => [...document.querySelectorAll<HTMLElement>("[data-direction]")];

const mousedown = async (el: Element, button = 0) => {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, button }));
    await vi.dynamicImportSettled();
  });
};

describe("ResizeHandles", () => {
  it("renders all 8 handles on Linux and starts a drag in each direction", async () => {
    render(<ResizeHandles platform="linux" />);
    const expected = ["North", "South", "East", "West", "NorthWest", "NorthEast", "SouthWest", "SouthEast"];
    expect(handles().map((h) => h.dataset.direction)).toEqual(expected);
    for (const handle of handles()) {
      await mousedown(handle);
      expect(startResizeDragging).toHaveBeenCalledWith(handle.dataset.direction);
    }
    expect(startResizeDragging).toHaveBeenCalledTimes(8);
  });

  it("ignores a right-button mousedown", async () => {
    render(<ResizeHandles platform="linux" />);
    for (const handle of handles()) await mousedown(handle, 2);
    expect(startResizeDragging).not.toHaveBeenCalled();
  });

  it("renders nothing on Windows", () => {
    render(<ResizeHandles platform="windows" />);
    expect(handles()).toHaveLength(0);
  });

  it("renders nothing outside Tauri", () => {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const root = createRoot(host);
    act(() => root.render(<ResizeHandles platform="linux" />));
    unmount = () => {
      root.unmount();
      host.remove();
    };
    expect(handles()).toHaveLength(0);
  });
});
