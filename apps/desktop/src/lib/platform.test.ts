import { describe, expect, it } from "vitest";
import { defaultRightClick, defaultTerminalFont } from "./platform";

describe("defaultTerminalFont", () => {
  it("is a font each platform has", () => {
    expect(defaultTerminalFont("macos")).toBe("Menlo");
    expect(defaultTerminalFont("windows")).toBe("Cascadia Mono");
    expect(defaultTerminalFont("linux")).toBe("Cascadia Mono");
  });
});

describe("defaultRightClick", () => {
  it("opens a menu on macOS and copies or pastes elsewhere", () => {
    expect(defaultRightClick("macos")).toBe("menu");
    expect(defaultRightClick("windows")).toBe("copy_paste");
    expect(defaultRightClick("linux")).toBe("copy_paste");
  });
});
