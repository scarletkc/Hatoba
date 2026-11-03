import { describe, expect, it } from "vitest";
import { boxSelection, clipBox, dropBefore, dropLine, hostsInBox, type Place } from "./hostPointer";

const at = (id: string, left: number, top: number, width = 100, height = 40): Place => ({
  id,
  left,
  top,
  right: left + width,
  bottom: top + height,
});

// Two rows of cards, 100 wide with a gap of 10, the second row with one card.
const cards = [at("a", 0, 0), at("b", 110, 0), at("c", 220, 0), at("d", 0, 50)];
const rows = [at("a", 0, 0, 400), at("b", 0, 40, 400), at("c", 0, 80, 400)];

describe("a selection box", () => {
  it("selects the hosts it touches", () => {
    expect(hostsInBox(cards, { left: 90, top: 30, right: 120, bottom: 60 })).toEqual(["a", "b", "d"]);
    expect(hostsInBox(cards, { left: 101, top: 41, right: 109, bottom: 49 })).toEqual([]);
  });

  it("replaces, adds to, or switches the selection", () => {
    expect(boxSelection(["a", "b"], ["b", "c"], "replace")).toEqual(["b", "c"]);
    expect(boxSelection(["a", "b"], ["b", "c"], "add")).toEqual(["a", "b", "c"]);
    expect(boxSelection(["a", "b"], ["b", "c"], "toggle")).toEqual(["a", "c"]);
  });

  it("is drawn only where the list shows", () => {
    const shown = { left: 0, top: 30, right: 400, bottom: 300 };
    expect(clipBox({ left: -20, top: -50, right: 500, bottom: 900 }, shown)).toEqual(shown);
    const inside = { left: 10, top: 40, right: 50, bottom: 60 };
    expect(clipBox(inside, shown)).toEqual(inside);
    // Dragged up past the head.
    expect(clipBox({ left: 10, top: -50, right: 50, bottom: 60 }, shown)).toEqual({ ...inside, top: 30 });
    expect(clipBox({ left: 10, top: 0, right: 50, bottom: 30 }, shown)).toBeNull();
  });
});

describe("dropping hosts", () => {
  it("goes by the middle of the rows", () => {
    expect(dropBefore(rows, 300, 10, "rows")).toBe("a");
    expect(dropBefore(rows, 300, 30, "rows")).toBe("b");
    expect(dropBefore(rows, 300, 200, "rows")).toBeNull();
  });

  it("goes row by row among cards, then by the middle of each card", () => {
    expect(dropBefore(cards, 40, 20, "grid")).toBe("a");
    expect(dropBefore(cards, 170, 20, "grid")).toBe("c");
    // Past the last card of a row: the end of that row, before the next one.
    expect(dropBefore(cards, 300, 20, "grid")).toBe("d");
    expect(dropBefore(cards, 300, 70, "grid")).toBeNull();
  });

  it("shows the line at the end of the row the pointer is in", () => {
    expect(dropLine(cards, "d", 20, "grid", 10)).toEqual({ left: 324, right: 326, top: 0, bottom: 40 });
    expect(dropLine(cards, "d", 70, "grid", 10)).toEqual({ left: -6, right: -4, top: 50, bottom: 90 });
    expect(dropLine(cards, "b", 20, "grid", 10)).toEqual({ left: 104, right: 106, top: 0, bottom: 40 });
    expect(dropLine(rows, null, 200, "rows", 0)).toEqual({ left: 0, right: 400, top: 119, bottom: 121 });
  });
});
