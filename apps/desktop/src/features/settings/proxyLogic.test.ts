import { describe, expect, it } from "vitest";
import { authority, parseProxyUrl, proxySummary, validateProxy, type ProxyFormValues } from "./proxyLogic";

describe("parseProxyUrl", () => {
  it("reads SOCKS5 and HTTP URLs with and without credentials", () => {
    expect(parseProxyUrl("socks5://127.0.0.1:7890")).toEqual({
      kind: "socks5",
      address: "127.0.0.1",
      port: 7890,
      username: "",
      password: "",
    });
    expect(parseProxyUrl(" http://alice:p%40ss@proxy.example.com:3128/ ")).toEqual({
      kind: "http",
      address: "proxy.example.com",
      port: 3128,
      username: "alice",
      password: "p@ss",
    });
    expect(parseProxyUrl("SOCKS5H://[::1]")).toEqual({ kind: "socks5", address: "::1", port: null, username: "", password: "" });
    expect(parseProxyUrl("socks://bob@10.0.0.1:1080")?.username).toBe("bob");
  });

  it("leaves everything else alone", () => {
    expect(parseProxyUrl("127.0.0.1")).toBeNull();
    expect(parseProxyUrl("https://proxy.example.com:443")).toBeNull();
    expect(parseProxyUrl("socks4://127.0.0.1:1080")).toBeNull();
    expect(parseProxyUrl("socks5://host:70000")).toBeNull();
    expect(parseProxyUrl("http://host:3128/path")).toBeNull();
  });
});

describe("validateProxy", () => {
  const ok: ProxyFormValues = { name: "Clash", kind: "socks5", address: "127.0.0.1", port: "7890", username: "", password: "" };

  it("accepts a complete form", () => {
    expect(validateProxy(ok)).toEqual({});
  });

  it("names the wrong field", () => {
    expect(validateProxy({ ...ok, name: " " }).name).toBe("nameRequired");
    expect(validateProxy({ ...ok, address: "" }).address).toBe("addressRequired");
    expect(validateProxy({ ...ok, address: "a b" }).address).toBe("addressInvalid");
    expect(validateProxy({ ...ok, address: "https://proxy" }).address).toBe("addressInvalid");
    expect(validateProxy({ ...ok, address: "[::1]" }).address).toBeUndefined();
    expect(validateProxy({ ...ok, port: "0" }).port).toBe("portInvalid");
    expect(validateProxy({ ...ok, port: "1.5" }).port).toBe("portInvalid");
    expect(validateProxy({ ...ok, kind: "http", username: "a:b" }).username).toBe("usernameColon");
    expect(validateProxy({ ...ok, username: "a:b" }).username).toBeUndefined();
    // 128 two-byte characters are 256 bytes.
    expect(validateProxy({ ...ok, password: "é".repeat(128) }).password).toBe("tooLong");
  });
});

describe("summaries", () => {
  it("puts IPv6 addresses in brackets and shows the username", () => {
    expect(authority("::1", 1080)).toBe("[::1]:1080");
    expect(proxySummary({ kind: "http", address: "proxy", port: 3128, username: "kc" })).toBe("HTTP · kc@proxy:3128");
  });
});
