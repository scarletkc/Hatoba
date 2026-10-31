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

  it("keeps a closing parenthesis that belongs to the link", () => {
    expect(messageParts("Verify at https://example.com/verify_(alice).")).toEqual([
      { text: "Verify at " },
      { text: "https://example.com/verify_(alice)", href: "https://example.com/verify_(alice)" },
      { text: "." },
    ]);
    expect(firstLink("(see https://example.com/wiki/Foo_(bar))")).toBe("https://example.com/wiki/Foo_(bar)");
    expect(firstLink("https://example.com/a)b(c)")).toBe("https://example.com/a)b(c)");
    // A run as long as a banner can be stays fast.
    expect(firstLink(`https://example.com/a${")".repeat(16_000)}`)).toBe("https://example.com/a");
  });

  it("keeps the bracket of an IPv6 address", () => {
    expect(firstLink("Open http://[fd7a:115c:a1e0::1].")).toBe("http://[fd7a:115c:a1e0::1]");
    expect(firstLink("(at http://[::1])")).toBe("http://[::1]");
  });

  it("links a scheme in capitals", () => {
    expect(firstLink("Visit HTTPS://login.example.com/check now")).toBe("HTTPS://login.example.com/check");
  });

  it("links only web addresses", () => {
    expect(messageParts("Authorized use only. file:///etc/passwd javascript:alert(1)")).toEqual([
      { text: "Authorized use only. file:///etc/passwd javascript:alert(1)" },
    ]);
    expect(firstLink("Authorized use only.")).toBeNull();
  });
});
