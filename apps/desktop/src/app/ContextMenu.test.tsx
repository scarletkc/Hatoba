import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { ContextMenuHost } from "./ContextMenu";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  document.body.innerHTML = "";
});

function render(html: string) {
  const page = document.createElement("div");
  page.innerHTML = html;
  document.body.appendChild(page);
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<ContextMenuHost platform="windows" />));
  unmount = () => root.unmount();
}

/** Right-clicks the element and says whether the web view's own menu was kept from showing. */
function rightClick(el: Element): boolean {
  const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 });
  act(() => void el.dispatchEvent(e));
  return e.defaultPrevented;
}

const items = () => [...document.querySelectorAll("[role=menu] [role=menuitem]")].map((b) => `${b.textContent}${(b as HTMLButtonElement).disabled ? " (off)" : ""}`);

describe("ContextMenuHost", () => {
  it("offers editing in a text field", () => {
    render(`<input id="f" value="deploy@prod" />`);
    const field = document.getElementById("f") as HTMLInputElement;
    field.setSelectionRange(0, 6);
    expect(rightClick(field)).toBe(true);
    expect(items()).toEqual(["CutCtrl+X", "CopyCtrl+C", "PasteCtrl+V", "Select AllCtrl+A"]);
  });

  it("keeps a password from being cut or copied", () => {
    render(`<input id="f" type="password" value="secret" />`);
    const field = document.getElementById("f") as HTMLInputElement;
    field.setSelectionRange(0, 6);
    rightClick(field);
    expect(items()).toEqual(["CutCtrl+X (off)", "CopyCtrl+C (off)", "PasteCtrl+V", "Select AllCtrl+A"]);
  });

  it("shows nothing, not the web view's menu, elsewhere", () => {
    render(`<div id="d">Hosts</div>`);
    expect(rightClick(document.getElementById("d")!)).toBe(true);
    expect(document.querySelector("[role=menu]")).toBeNull();
  });

  it("leaves a menu of the app's own alone", () => {
    render(`<div id="d"><input id="f" value="x" /></div>`);
    document.getElementById("d")!.addEventListener("contextmenu", (e) => e.preventDefault());
    rightClick(document.getElementById("f")!);
    expect(document.querySelector("[role=menu]")).toBeNull();
  });
});
