import { describe, expect, it } from "vitest";
import { ALONE, pickHost } from "./selection";

const hosts = ["a", "b", "c", "d", "e"];

describe("pickHost", () => {
  it("selects a host alone without modifier keys", () => {
    expect(pickHost(hosts, ["a", "b"], "a", "d", ALONE)).toEqual(["d"]);
  });

  it("adds a host with Ctrl, or takes it out again", () => {
    expect(pickHost(hosts, ["b"], "b", "d", { range: false, add: true })).toEqual(["b", "d"]);
    expect(pickHost(hosts, ["b", "d"], "d", "b", { range: false, add: true })).toEqual(["d"]);
    expect(pickHost(hosts, ["b"], "b", "b", { range: false, add: true })).toEqual([]);
  });

  it("selects the hosts from the anchor with Shift, the one clicked last", () => {
    expect(pickHost(hosts, ["b"], "b", "d", { range: true, add: false })).toEqual(["b", "c", "d"]);
    expect(pickHost(hosts, ["d"], "d", "b", { range: true, add: false })).toEqual(["c", "d", "b"]);
    // A second Shift+click starts from the same anchor and leaves out what the first took in.
    expect(pickHost(hosts, ["b", "c", "d", "e"], "b", "c", { range: true, add: false })).toEqual(["b", "c"]);
  });

  it("adds the range to the selection with Ctrl+Shift", () => {
    expect(pickHost(hosts, ["a", "d"], "d", "e", { range: true, add: true })).toEqual(["a", "d", "e"]);
  });

  it("selects only the host clicked when the anchor is not listed", () => {
    expect(pickHost(hosts, [], "gone", "c", { range: true, add: false })).toEqual(["c"]);
  });

  it("drops quick connect rows when several are picked", () => {
    expect(pickHost(hosts, ["quick:typed"], "quick:typed", "b", { range: false, add: true })).toEqual(["b"]);
  });
});
