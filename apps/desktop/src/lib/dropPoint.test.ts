import { describe, expect, it } from "vitest";
import { dropPointToCss, isDropInside } from "./dropPoint";

const rect = { left: 100, top: 50, right: 300, bottom: 250 };

describe("dropPointToCss", () => {
  it("divides Windows physical pixels by the device pixel ratio", () => {
    expect(dropPointToCss({ x: 400, y: 200 }, "windows", 2)).toEqual({ x: 200, y: 100 });
    expect(dropPointToCss({ x: 375, y: 150 }, "windows", 1.5)).toEqual({ x: 250, y: 100 });
  });

  it("keeps macOS points as they are, on Retina too", () => {
    expect(dropPointToCss({ x: 200, y: 100 }, "macos", 2)).toEqual({ x: 200, y: 100 });
  });

  it("treats a missing device pixel ratio as 1", () => {
    expect(dropPointToCss({ x: 200, y: 100 }, "windows", 0)).toEqual({ x: 200, y: 100 });
  });
});

describe("isDropInside", () => {
  it("tests the converted point against the rectangle", () => {
    expect(isDropInside(rect, { x: 400, y: 200 }, "windows", 2)).toBe(true);
    expect(isDropInside(rect, { x: 400, y: 200 }, "macos", 2)).toBe(false);
    expect(isDropInside(rect, { x: 200, y: 100 }, "macos", 2)).toBe(true);
    expect(isDropInside(rect, { x: 90, y: 100 }, "macos", 2)).toBe(false);
  });

  it("is false without a rectangle", () => {
    expect(isDropInside(undefined, { x: 0, y: 0 }, "macos", 2)).toBe(false);
  });
});
