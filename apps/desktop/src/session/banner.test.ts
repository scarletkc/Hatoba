import { describe, expect, it } from "vitest";
import { firstLink, messageParts } from "./banner";

describe("server messages", () => {
  it("splits the links out of the text", () => {
    const text =
      "# Tailscale SSH requires an additional check.\n# To authenticate, visit: https://login.tailscale.com/a/1234";
    expect(messageParts(text)).toEqual([
      { text: "# Tailscale SSH requires an additional check.\n# To authenticate, visit: " },
      { text: "https://login.tailscale.com/a/1234", href: "https://login.tailscale.com/a/1234" },
    ]);
    expect(firstLink(text)).toBe("https://login.tailscale.com/a/1234");
  });

  it("leaves the punctuation after a link in the text", () => {
    expect(messageParts("See https://example.com/policy. Or (http://example.org).")).toEqual([
      { text: "See " },
      { text: "https://example.com/policy", href: "https://example.com/policy" },
      { text: ". Or (" },
      { text: "http://example.org", href: "http://example.org" },
      { text: ")." },
    ]);
  });

  it("links only web addresses", () => {
    expect(messageParts("Authorized use only. file:///etc/passwd javascript:alert(1)")).toEqual([
      { text: "Authorized use only. file:///etc/passwd javascript:alert(1)" },
    ]);
    expect(firstLink("Authorized use only.")).toBeNull();
  });
});
