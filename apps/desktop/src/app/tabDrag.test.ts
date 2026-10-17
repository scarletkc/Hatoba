import { beforeEach, describe, expect, it } from "vitest";
import { dropIndex, shiftOf, type Slot } from "./tabDrag";
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
