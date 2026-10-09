import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { OsIcon } from "./OsIcon";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(ui: React.ReactNode): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host.firstElementChild as HTMLElement;
}

describe("OsIcon", () => {
  it("names a known OS", () => {
    const icon = render(<OsIcon os="raspbian" badge="red" />);
    expect(icon.getAttribute("role")).toBe("img");
    expect(icon.getAttribute("aria-label")).toBe("Raspbian");
    expect(icon.title).toBe("Raspbian");
    expect(icon.querySelector(".ph-hard-drives")).toBeNull();
  });

  it("draws Windows with the Phosphor logo", () => {
    const icon = render(<OsIcon os="windows" badge="red" />);
    expect(icon.querySelector(".ph-fill.ph-windows-logo")).not.toBeNull();
  });

  it.each([null, "fedora", "toString"])("shows a generic server for %s", (os) => {
    const icon = render(<OsIcon os={os} badge="red" />);
    expect(icon.querySelector(".ph-hard-drives")).not.toBeNull();
    expect(icon.getAttribute("aria-hidden")).toBe("true");
    expect(icon.title).toBe("");
  });

  it("paints the status badge", () => {
    const icon = render(<OsIcon os={null} badge="red" />);
    expect((icon.lastElementChild as HTMLElement).style.background).toBe("red");
  });
});
