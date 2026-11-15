import { describe, expect, it } from "vitest";
import { OSC52_MAX_BASE64, osc52Text } from "./osc52";

const base64 = (text: string) => btoa(String.fromCharCode(...new TextEncoder().encode(text)));

describe("OSC 52 clipboard text (TERM-04)", () => {
  it.each(["c", "", "pc", "cs0"])("reads the text for the selection %j", (selections) => {
    expect(osc52Text(`${selections};${base64("hello")}`)).toBe("hello");
  });

  it("decodes UTF-8 and keeps line breaks", () => {
    expect(osc52Text(`c;${base64("日本語\n✓ done")}`)).toBe("日本語\n✓ done");
  });

  it("reads base64 without padding", () => {
    expect(osc52Text("c;aGk")).toBe("hi");
  });

  it.each(["c;?", ";?"])("does not answer the query %j", (data) => {
    expect(osc52Text(data)).toBeNull();
  });

  it.each(["p", "s", "q0"])("leaves the selection %j alone", (selections) => {
    expect(osc52Text(`${selections};${base64("hello")}`)).toBeNull();
  });

  it("takes text up to the limit and none past it", () => {
    expect(osc52Text(`c;${"QUFB".repeat(OSC52_MAX_BASE64 / 4)}`)).toHaveLength((OSC52_MAX_BASE64 / 4) * 3);
    expect(osc52Text(`c;${"QUFB".repeat(OSC52_MAX_BASE64 / 4 + 1)}`)).toBeNull();
  });

  it.each(["", "c", "c;", "c;not base64!", "c;a"])("has nothing to copy in %j", (data) => {
    expect(osc52Text(data)).toBeNull();
  });
});
