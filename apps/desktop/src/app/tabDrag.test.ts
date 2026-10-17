import type { PointerEvent as ReactPointerEvent } from "react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { TabDrag, dropIndex, shiftOf, type Slot } from "./tabDrag";
import { useTabs } from "./tabs";

// Three tabs 100, 60 and 80 px wide, 2 px apart.
const slots: Slot[] = [
  { left: 0, width: 100 },
  { left: 102, width: 60 },
  { left: 164, width: 80 },
];

describe("dropIndex", () => {
  it("moves right once the right edge passes a neighbour's middle", () => {
    expect(dropIndex(slots, 0, 0)).toBe(0);
    // The right edge at 131 is short of the second tab's middle at 132.
    expect(dropIndex(slots, 0, 31)).toBe(0);
    expect(dropIndex(slots, 0, 33)).toBe(1);
    // Held against the end of the strip, the wide tab passes the narrower last one.
    expect(dropIndex(slots, 0, 144)).toBe(2);
  });

  it("moves left once the left edge passes a neighbour's middle", () => {
    expect(dropIndex(slots, 2, 164)).toBe(2);
    expect(dropIndex(slots, 2, 133)).toBe(2);
    expect(dropIndex(slots, 2, 131)).toBe(1);
    expect(dropIndex(slots, 2, 51)).toBe(1);
    expect(dropIndex(slots, 2, 49)).toBe(0);
  });
});

describe("shiftOf", () => {
  it("moves the tabs passed over by the dragged tab's step", () => {
    expect([0, 1, 2, 3].map((i) => shiftOf(i, 0, 2, 102))).toEqual([0, -102, -102, 0]);
    expect([0, 1, 2, 3].map((i) => shiftOf(i, 3, 1, 62))).toEqual([0, 62, 62, 0]);
    expect([0, 1, 2].map((i) => shiftOf(i, 1, 1, 62))).toEqual([0, 0, 0]);
  });
});

describe("moveTab", () => {
  const ids = () => useTabs.getState().tabs.map((t) => t.title);

  beforeEach(() => {
    useTabs.setState({ tabs: [], active: "home" });
    for (const title of ["a", "b", "c"]) useTabs.getState().openSession({ hostId: title, target: null }, title);
  });

  it("moves a tab to an index among the session tabs", () => {
    const [a, , c] = useTabs.getState().tabs;
    useTabs.getState().moveTab(a.id, 2);
    expect(ids()).toEqual(["b", "c", "a"]);
    useTabs.getState().moveTab(c.id, 0);
    expect(ids()).toEqual(["c", "b", "a"]);
  });

  it("leaves the tabs alone for their own place or an unknown tab", () => {
    const before = useTabs.getState().tabs;
    useTabs.getState().moveTab(before[1].id, 1);
    useTabs.getState().moveTab("gone", 0);
    expect(useTabs.getState().tabs).toBe(before);
  });
});

describe("TabDrag", () => {
  let strip: HTMLElement;
  let drag: TabDrag;
  const el = (title: string) => strip.querySelector<HTMLElement>(`[data-title="${title}"]`)!;
  const tabOf = (title: string) => useTabs.getState().tabs.find((t) => t.title === title)!;
  const press = (title: string, target: Element = el(title)) =>
    drag.down({ button: 0, target, currentTarget: el(title), pointerId: 1, clientX: 10 } as unknown as ReactPointerEvent<HTMLElement>, tabOf(title).id);
  const moveTo = (clientX: number) => {
    const e = new MouseEvent("pointermove", { clientX });
    Object.defineProperty(e, "pointerId", { value: 1 });
    window.dispatchEvent(e);
  };

  beforeEach(() => {
    useTabs.setState({ tabs: [], active: "home" });
    strip = document.createElement("div");
    for (const title of ["a", "b"]) {
      const id = useTabs.getState().openSession({ hostId: title, target: null }, title);
      strip.insertAdjacentHTML("beforeend", `<button data-tab-id="${id}" data-title="${title}"><span role="button">x</span></button>`);
    }
    document.body.append(strip);
    useTabs.getState().activate("home");
    drag = new TabDrag();
  });

  afterEach(() => {
    drag.dispose();
    strip.remove();
  });

  it("brings a pressed tab to the front, but not for its close button", () => {
    press("a", el("a").querySelector("span")!);
    expect(useTabs.getState().active).toBe("home");
    press("b");
    expect(useTabs.getState().active).toBe(tabOf("b").id);
  });

  it("ends a press when its tab closes before the drag starts", () => {
    press("b");
    useTabs.getState().closeTab(tabOf("b").id);
    el("b").remove();
    drag.rendered(useTabs.getState().tabs);
    expect(() => moveTo(40)).not.toThrow();
    expect(strip.querySelector("[data-dragging]")).toBeNull();
    press("a");
    moveTo(40);
    expect(el("a").hasAttribute("data-dragging")).toBe(true);
  });

  it("does not drag a tab that left the strip", () => {
    press("b");
    const gone = el("b");
    gone.remove();
    expect(() => moveTo(40)).not.toThrow();
    expect(gone.hasAttribute("data-dragging")).toBe(false);
    // The press is over, so a new one starts a drag.
    press("a");
    moveTo(40);
    expect(el("a").hasAttribute("data-dragging")).toBe(true);
  });
});
