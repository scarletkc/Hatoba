import { describe, expect, it } from "vitest";
import { reachableAddress } from "./address";

describe("where a forward is reached on this computer", () => {
  it("is localhost for loopback and wildcard addresses", () => {
    for (const bind of ["127.0.0.1", "127.0.1.1", "localhost", "0.0.0.0", "::", "::1", ""]) {
      expect(reachableAddress(bind, 5433)).toBe("localhost:5433");
    }
  });

  it("is the address an interface listens on", () => {
    expect(reachableAddress("192.168.1.10", 5433)).toBe("192.168.1.10:5433");
    expect(reachableAddress("fd00::5", 5433)).toBe("[fd00::5]:5433");
  });
});
