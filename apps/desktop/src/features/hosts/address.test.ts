import { describe, expect, it } from "vitest";
import { translate, type MessageKey, type Params } from "@/i18n";
import type { ForwardView } from "@/ipc/types";
import { forwardRule, reachableAddress } from "./address";

describe("where a forward is reached on this computer", () => {
  it("is localhost for the addresses localhost names and wildcard ones", () => {
    for (const bind of ["127.0.0.1", "localhost", "0.0.0.0", "::", "::1", ""]) {
      expect(reachableAddress(bind, 5433)).toBe("localhost:5433");
    }
  });

  it("is the address an interface listens on", () => {
    expect(reachableAddress("127.0.1.1", 5433)).toBe("127.0.1.1:5433");
    expect(reachableAddress("192.168.1.10", 5433)).toBe("192.168.1.10:5433");
    expect(reachableAddress("fd00::5", 5433)).toBe("[fd00::5]:5433");
  });
});

describe("a forward as one line", () => {
  const t = (key: MessageKey, params?: Params) => translate("en", key, params);
  const forward = (patch: Partial<ForwardView>): ForwardView => ({
    id: "f1",
    host_id: "h1",
    kind: "local",
    bind_address: "127.0.0.1",
    bind_port: 5433,
    dest_host: "db.internal",
    dest_port: 5432,
    auto_start: false,
    ...patch,
  });

  it("names where each kind listens and where it leads", () => {
    expect(forwardRule(t, forward({}))).toBe("127.0.0.1:5433 → db.internal:5432");
    expect(forwardRule(t, forward({ kind: "remote", bind_address: "localhost", bind_port: 8080, dest_host: "::1", dest_port: 3000 }))).toBe(
      "localhost:8080 on the server → [::1]:3000",
    );
    expect(forwardRule(t, forward({ kind: "dynamic", bind_port: 1080, dest_host: "", dest_port: 0 }))).toBe("127.0.0.1:1080 (SOCKS proxy)");
  });

  it("shows the port picked for port 0", () => {
    expect(forwardRule(t, forward({ bind_port: 0 }))).toBe("127.0.0.1:auto → db.internal:5432");
    expect(forwardRule(t, forward({ kind: "remote", bind_port: 0 }), 40123)).toBe("127.0.0.1:40123 on the server → db.internal:5432");
  });
});
