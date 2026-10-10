import { describe, expect, it } from "vitest";
import type { TabSource } from "@/app/tabs";
import { translate, type MessageKey, type Params } from "@/i18n";
import type { HostView, QuickTarget } from "@/ipc/types";
import { connectionFor, conversationPlace, movedFrom, placeName } from "./place";

const t = (key: MessageKey, params?: Params) => translate("en", key, params);
const host = (id: string, name: string, address: string, port = 22, username = "root") => ({ id, name, address, port, username }) as HostView;
const hosts = [
  host("h-prod", "prod-db", "10.0.0.5"),
  host("h-web", "web-1", "10.0.0.9"),
  host("h-v6", "v6", "2001:db8::1", 2222),
  host("h-nas", "nas", "NAS.local"),
  host("h-web-2", "web-1", "10.0.0.10"),
];

const target = (address: string, port = 22, username = "root"): QuickTarget => ({ address, port, username });
const onHost = (host_id: string) => ({ host_id, quick_target: null });
/** As Rust records it: the address in lowercase and without brackets. */
const onTarget = (quick_target: QuickTarget) => ({ host_id: null, quick_target });
const homeChat = { host_id: null, quick_target: null };
const hostTab = (hostId: string): TabSource => ({ hostId, target: null });
const quickTab = (address: string, port = 22, username = "root"): TabSource => ({ hostId: null, target: target(address, port, username) });

describe("conversationPlace (AI-23, AI-25)", () => {
  it("names a host, a deleted host, a quick-connect target, or none", () => {
    expect(placeName(t, conversationPlace(onHost("h-prod"), hosts))).toBe("prod-db");
    expect(placeName(t, conversationPlace(onHost("h-gone"), hosts))).toBe("Deleted host");
    expect(placeName(t, conversationPlace(onTarget(target("2001:db8::1", 2222)), hosts))).toBe("root@[2001:db8::1]:2222");
    expect(placeName(t, conversationPlace(homeChat, hosts))).toBe("No host");
  });
});

describe("movedFrom (AI-09)", () => {
  it("names the host a conversation moves off, empty when it was deleted", () => {
    expect(movedFrom(onHost("h-prod"), hostTab("h-web"), hosts)).toEqual({ from: "prod-db", to: "web-1" });
    expect(movedFrom(onHost("h-gone"), hostTab("h-web"), hosts)).toEqual({ from: "", to: "web-1" });
    expect(movedFrom(onHost("h-prod"), quickTab("10.0.0.7"), hosts)).toEqual({ from: "prod-db", to: "root@10.0.0.7:22" });
    expect(movedFrom(onHost("h-prod"), hostTab("h-prod"), hosts)).toBeNull();
  });

  it("names the quick-connect target a conversation moves off (HOST-12)", () => {
    const on7 = onTarget(target("10.0.0.7"));
    expect(movedFrom(on7, hostTab("h-prod"), hosts)).toEqual({ from: "root@10.0.0.7:22", to: "prod-db" });
    expect(movedFrom(on7, quickTab("10.0.0.8"), hosts)).toEqual({ from: "root@10.0.0.7:22", to: "root@10.0.0.8:22" });
    expect(movedFrom(on7, quickTab("10.0.0.7", 2222), hosts)).toEqual({ from: "root@10.0.0.7:22", to: "root@10.0.0.7:2222" });
    expect(movedFrom(on7, quickTab("10.0.0.7"), hosts)).toBeNull();
    // The tab's target is named as Rust records it, the address in lowercase.
    expect(movedFrom(on7, quickTab("DB.Example"), hosts)).toEqual({ from: "root@10.0.0.7:22", to: "root@db.example:22" });
  });

  it("stays on a target whose user name has a space", () => {
    const user = "domain user";
    expect(movedFrom(onTarget(target("10.0.0.7", 22, user)), quickTab("10.0.0.7", 22, user), hosts)).toBeNull();
    expect(movedFrom(onTarget(target("10.0.0.7", 22, user)), hostTab("h-prod"), hosts)).toEqual({ from: "domain user@10.0.0.7:22", to: "prod-db" });
  });

  it("does not note a move between a target and the host saved from it", () => {
    expect(movedFrom(onTarget(target("10.0.0.9")), hostTab("h-web"), hosts)).toBeNull();
    expect(movedFrom(onHost("h-web"), quickTab("10.0.0.9"), hosts)).toBeNull();
    expect(movedFrom(onTarget(target("2001:db8::1", 2222)), hostTab("h-v6"), hosts)).toBeNull();
    expect(movedFrom(onHost("h-v6"), quickTab("[2001:DB8::1]", 2222), hosts)).toBeNull();
    // Addresses compare in any letter case, as quick connect compares them (HOST-12).
    expect(movedFrom(onTarget(target("nas.local")), hostTab("h-nas"), hosts)).toBeNull();
    expect(movedFrom(onHost("h-nas"), quickTab("nas.local"), hosts)).toBeNull();
    expect(movedFrom(onTarget(target("nas.local")), quickTab("NAS.local"), hosts)).toBeNull();
  });

  it("names nothing where Rust writes no note", () => {
    // A home tab chat has no host to come from.
    expect(movedFrom(homeChat, hostTab("h-prod"), hosts)).toBeNull();
    expect(movedFrom(homeChat, quickTab("10.0.0.7"), hosts)).toBeNull();
    // The tab's saved host was deleted, so the note would name nothing to move to.
    expect(movedFrom(onHost("h-prod"), hostTab("h-gone"), hosts)).toBeNull();
    expect(movedFrom(onTarget(target("10.0.0.7")), hostTab("h-gone"), hosts)).toBeNull();
    // Two hosts with one name.
    expect(movedFrom(onHost("h-web"), hostTab("h-web-2"), hosts)).toBeNull();
  });
});

describe("connectionFor (AI-09)", () => {
  it("opens the conversation's host", () => {
    expect(connectionFor(onHost("h-prod"), hosts)).toEqual({ name: "prod-db", source: hostTab("h-prod") });
    expect(connectionFor(onHost("h-gone"), hosts)).toBeNull();
    expect(connectionFor(homeChat, hosts)).toBeNull();
  });

  it("opens its quick-connect target, as the host saved from it when there is one", () => {
    expect(connectionFor(onTarget(target("10.0.0.7", 2222)), hosts)).toEqual({ name: "root@10.0.0.7:2222", source: quickTab("10.0.0.7", 2222) });
    expect(connectionFor(onTarget(target("10.0.0.7", 22, "domain user")), hosts)).toEqual({
      name: "domain user@10.0.0.7:22",
      source: quickTab("10.0.0.7", 22, "domain user"),
    });
    expect(connectionFor(onTarget(target("10.0.0.9")), hosts)).toEqual({ name: "web-1", source: hostTab("h-web") });
    expect(connectionFor(onTarget(target("2001:db8::1", 2222)), hosts)).toEqual({ name: "v6", source: hostTab("h-v6") });
  });
});
