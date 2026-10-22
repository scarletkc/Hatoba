import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { confirm, ConfirmHost, Menu, Modal, setOverlaysLocked, useMenu } from "./overlay";

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

describe("Menu", () => {
  const key = (k: string) => act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: k })));
  const click = (el: Element | null | undefined) => act(() => (el as HTMLElement).click());
  const radios = () => [...document.querySelectorAll("[role=group] [role=menuitemradio]")];
  const checked = () => radios().map((b) => b.getAttribute("aria-checked"));

  /** A model item with the choice row under it, as the AI panel's model selector has. */
  function Picker({ onClose, onSelect }: { onClose: () => void; onSelect: () => void }) {
    const [level, setLevel] = useState("default");
    return (
      <Menu
        anchor={{ x: 0, y: 0 }}
        onClose={onClose}
        entries={[
          { label: "Model", checked: true, onSelect },
          {
            kind: "choice",
            label: "Thinking Level",
            options: ["default", "low", "high"].map((value) => ({ value, label: value.toUpperCase() })),
            value: level,
            onChange: setLevel,
          },
          { label: "Keep", checked: false, keepOpen: true, onSelect },
        ]}
      />
    );
  }

  it("picks an option of a choice row without closing", () => {
    const onClose = vi.fn();
    const onSelect = vi.fn();
    render(<Picker onClose={onClose} onSelect={onSelect} />);
    expect(document.querySelector("[role=group]")?.getAttribute("aria-label")).toBe("Thinking Level");
    expect(checked()).toEqual(["true", "false", "false"]);
    click(radios()[2]);
    expect(checked()).toEqual(["false", "false", "true"]);
    expect(onClose).not.toHaveBeenCalled();

    // Keyboard: down to the row, left and right move the choice, Enter closes.
    key("ArrowDown");
    key("ArrowDown");
    key("ArrowLeft");
    expect(checked()).toEqual(["false", "true", "false"]);
    key("ArrowLeft");
    key("ArrowLeft");
    expect(checked()).toEqual(["true", "false", "false"]);
    key("Enter");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("closes on Escape without closing the sheet around it", () => {
    const onClose = vi.fn();
    const onSheetClose = vi.fn();
    render(
      <Modal onClose={onSheetClose}>
        <button type="button">Field</button>
        <Picker onClose={onClose} onSelect={() => {}} />
      </Modal>,
    );
    key("Escape");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSheetClose).not.toHaveBeenCalled();
  });

  it("stays open for a keep-open item", () => {
    const onClose = vi.fn();
    const onSelect = vi.fn();
    render(<Picker onClose={onClose} onSelect={onSelect} />);
    const keep = document.querySelectorAll("[role=menuitemcheckbox]");
    expect(keep).toHaveLength(1);
    click(keep[0]);
    expect(onSelect).toHaveBeenCalledTimes(1);
    expect(onClose).not.toHaveBeenCalled();
    click(document.querySelector("[role=menu] > [role=menuitemradio]"));
    expect(onSelect).toHaveBeenCalledTimes(2);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("goes to the last item on ↑ from nothing highlighted", () => {
    const onSelect = vi.fn();
    render(
      <Menu
        anchor={{ x: 0, y: 0 }}
        onClose={() => {}}
        entries={[
          { label: "System", onSelect: () => onSelect("System") },
          { kind: "separator" },
          { label: "English", onSelect: () => onSelect("English") },
          { label: "日本語", onSelect: () => onSelect("日本語") },
        ]}
      />,
    );
    key("ArrowUp");
    key("Enter");
    expect(onSelect).toHaveBeenCalledWith("日本語");
  });

  it("passes over disabled items with the arrow keys", () => {
    const onSelect = vi.fn();
    render(
      <Menu
        anchor={{ x: 0, y: 0 }}
        onClose={() => {}}
        entries={[
          { label: "Copy", disabled: true, onSelect: () => onSelect("Copy") },
          { label: "Paste", onSelect: () => onSelect("Paste") },
          { kind: "separator" },
          { label: "Select All", disabled: true, onSelect: () => onSelect("Select All") },
        ]}
      />,
    );
    key("ArrowUp");
    key("Enter");
    expect(onSelect).toHaveBeenCalledWith("Paste");
  });

  it("drops the highlight when the pointer leaves the item", () => {
    const onSelect = vi.fn();
    render(<Menu anchor={{ x: 0, y: 0 }} onClose={() => {}} entries={[{ label: "Item", onSelect }]} />);
    const item = document.querySelector("[role=menuitem]")!;
    const pointer = (type: string) => act(() => void item.dispatchEvent(new MouseEvent(type, { bubbles: true, relatedTarget: document.body })));
    pointer("mouseover");
    pointer("mouseout");
    key("Enter");
    expect(onSelect).not.toHaveBeenCalled();
    pointer("mouseover");
    key("Enter");
    expect(onSelect).toHaveBeenCalledTimes(1);
  });

  /** The menu's layout size, which placement reads (jsdom lays nothing out). */
  const menuSize = (width: number, height: number) => {
    const w = vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockReturnValue(width);
    const h = vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockReturnValue(height);
    return {
      mockRestore: () => {
        w.mockRestore();
        h.mockRestore();
      },
    };
  };

  it("opens above its button, not over it, when it doesn't fit below", () => {
    const rect = menuSize(200, 120);
    try {
      render(<Menu anchor={{ x: 0, y: window.innerHeight - 20, above: 600 }} onClose={() => {}} entries={[{ label: "Item", onSelect: () => {} }]} />);
      expect((document.querySelector("[role=menu]") as HTMLElement).style.top).toBe("480px");
    } finally {
      rect.mockRestore();
    }
  });

  it("stays inside the painted window surface, not its transparent margin, while floating", () => {
    const rect = menuSize(200, 120);
    const root = document.documentElement;
    const anchor = { x: window.innerWidth - 10, y: window.innerHeight - 140 };
    const pos = () => {
      const { left, top } = (document.querySelector("[role=menu]") as HTMLElement).style;
      return { left, top };
    };
    try {
      render(<Menu anchor={anchor} onClose={() => {}} entries={[{ label: "Item", onSelect: () => {} }]} />);
      expect(pos()).toEqual({ left: `${window.innerWidth - 208}px`, top: `${window.innerHeight - 140}px` });
      act(() => unmount?.());

      root.dataset.floating = "";
      root.style.setProperty("--win-pad", "22px");
      render(<Menu anchor={anchor} onClose={() => {}} entries={[{ label: "Item", onSelect: () => {} }]} />);
      // 8px inside the surface's right edge, and flipped up since the bottom would land in the margin.
      expect(pos()).toEqual({ left: `${window.innerWidth - 230}px`, top: `${window.innerHeight - 268}px` });
    } finally {
      rect.mockRestore();
      delete root.dataset.floating;
      root.style.removeProperty("--win-pad");
    }
  });

  it("closes when its button is pressed again", () => {
    function Trigger() {
      const menu = useMenu();
      return (
        <>
          <button type="button" id="trigger" onClick={(e) => menu.openBelow(e.currentTarget)}>
            Open
          </button>
          {menu.anchor && <Menu anchor={menu.anchor} onClose={menu.close} entries={[{ label: "Item", onSelect: () => {} }]} />}
        </>
      );
    }
    render(<Trigger />);
    const trigger = document.getElementById("trigger")!;
    const press = () =>
      act(() => {
        trigger.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        trigger.click();
      });
    press();
    expect(document.querySelector("[role=menu]")).not.toBeNull();
    press();
    expect(document.querySelector("[role=menu]")).toBeNull();
    press();
    expect(document.querySelector("[role=menu]")).not.toBeNull();
    act(() => void document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true })));
    expect(document.querySelector("[role=menu]")).toBeNull();
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

  it("shows a modal opened on the lock screen", () => {
    act(() => setOverlaysLocked(true));
    const onClose = vi.fn();
    render(
      <Modal whileLocked onClose={onClose}>
        <input defaultValue="Recovery code" />
      </Modal>,
    );
    expect(document.body.querySelector("input")?.value).toBe("Recovery code");
    expect(document.activeElement?.tagName).toBe("INPUT");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(onClose).toHaveBeenCalledTimes(1);
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
