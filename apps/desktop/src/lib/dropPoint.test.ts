import { describe, expect, it } from "vitest";
import { dropPointToCss, isDropInside } from "./dropPoint";

const rect = { left: 100, top: 50, right: 300, bottom: 250 };

describe("dropPointToCss", () => {
  it("divides Windows physical pixels by the device pixel ratio", () => {
    expect(dropPointToCss({ x: 400, y: 200 }, "windows", 2, 1)).toEqual({ x: 200, y: 100 });
    expect(dropPointToCss({ x: 375, y: 150 }, "windows", 1.5, 1)).toEqual({ x: 250, y: 100 });
  });

  it("leaves the interface zoom on Windows to the device pixel ratio, which holds it", () => {
    // 150% display scale at 200% zoom.
    expect(dropPointToCss({ x: 600, y: 300 }, "windows", 3, 2)).toEqual({ x: 200, y: 100 });
  });

  it("keeps macOS points as they are, on Retina too", () => {
    expect(dropPointToCss({ x: 200, y: 100 }, "macos", 2, 1)).toEqual({ x: 200, y: 100 });
  });

  it("keeps Linux logical pixels as they are under display scaling", () => {
    expect(dropPointToCss({ x: 200, y: 100 }, "linux", 2, 1)).toEqual({ x: 200, y: 100 });
  });

  it("divides macOS and Linux view pixels by the interface zoom alone", () => {
    // WebKit's devicePixelRatio is 3 here: a Retina display at 150% zoom.
    expect(dropPointToCss({ x: 300, y: 150 }, "macos", 3, 1.5)).toEqual({ x: 200, y: 100 });
    expect(dropPointToCss({ x: 134, y: 67 }, "linux", 0.67, 0.67)).toEqual({ x: 200, y: 100 });
  });

  it("treats a missing device pixel ratio or zoom as 1", () => {
    expect(dropPointToCss({ x: 200, y: 100 }, "windows", 0, 1)).toEqual({ x: 200, y: 100 });
    expect(dropPointToCss({ x: 200, y: 100 }, "macos", 2, 0)).toEqual({ x: 200, y: 100 });
  });
});

describe("isDropInside", () => {
  it("tests the converted point against the rectangle", () => {
    expect(isDropInside(rect, { x: 400, y: 200 }, "windows", 2, 1)).toBe(true);
    expect(isDropInside(rect, { x: 400, y: 200 }, "macos", 2, 1)).toBe(false);
    expect(isDropInside(rect, { x: 400, y: 200 }, "linux", 2, 1)).toBe(false);
    expect(isDropInside(rect, { x: 200, y: 100 }, "macos", 2, 1)).toBe(true);
    expect(isDropInside(rect, { x: 90, y: 100 }, "macos", 2, 1)).toBe(false);
    expect(isDropInside(rect, { x: 400, y: 200 }, "macos", 4, 2)).toBe(true);
  });

  it("is false without a rectangle", () => {
    expect(isDropInside(undefined, { x: 0, y: 0 }, "macos", 2, 1)).toBe(false);
  });
});
