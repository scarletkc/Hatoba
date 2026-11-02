import { describe, expect, it } from "vitest";
import { moveHosts, orderHosts, sortHosts, type HostLike } from "./order";

const host = (id: string, sort = 0): HostLike & { id: string } => ({
  id,
  name: id,
  address: `${id}.example`,
  port: 22,
  username: "ops",
  tags: [],
  favorite: false,
  group_id: null,
  last_connected_at: null,
  sort,
});

const ids = (hosts: { id: string }[]) => hosts.map((h) => h.id);

describe("the custom order", () => {
  it("goes by place, with the hosts that have none last by name", () => {
    const hosts = [host("c"), host("b", 20), host("a"), host("d", 10)];
    expect(ids(sortHosts(hosts, "custom", "en"))).toEqual(["d", "b", "a", "c"]);
  });

  it("shows an order just made until the hosts have their new places", () => {
    const hosts = [host("a", 10), host("b", 20), host("c", 30)];
    expect(ids(orderHosts(hosts, "custom", "en", ["c", "a", "b"]))).toEqual(["c", "a", "b"]);
    expect(ids(orderHosts(hosts, "name", "en", ["c", "a", "b"]))).toEqual(["a", "b", "c"]);
  });
});

describe("moveHosts", () => {
  const order = ["a", "b", "c", "d", "e"];

  it("moves hosts before another, keeping their order", () => {
    expect(moveHosts(order, order, ["d", "b"], "a")).toEqual(["b", "d", "a", "c", "e"]);
  });

  it("moves hosts after the last host listed that stays", () => {
    // A group page lists b and d: dropping b at its end puts it right after d, before e.
    expect(moveHosts(order, ["b", "d"], ["b"], null)).toEqual(["a", "c", "d", "b", "e"]);
  });

  it("leaves the order alone when every listed host moves", () => {
    expect(moveHosts(order, ["b"], ["b"], null)).toEqual(order);
  });
});
