import { describe, expect, it } from "vitest";
import type { TabSource } from "@/app/tabs";
import { translate, type MessageKey, type Params } from "@/i18n";
import type { HostView } from "@/ipc/types";
import { connectionFor, conversationPlace, movedFrom, placeName } from "./place";

const t = (key: MessageKey, params?: Params) => translate("en", key, params);
const host = (id: string, name: string, address: string, port = 22, username = "root") => ({ id, name, address, port, username }) as HostView;
const hosts = [host("h-prod", "prod-db", "10.0.0.5"), host("h-web", "web-1", "10.0.0.9"), host("h-v6", "v6", "2001:db8::1", 2222), host("h-nas", "nas", "NAS.local")];

const onHost = (host_id: string) => ({ host_id, quick_target: null });
const onTarget = (quick_target: string) => ({ host_id: null, quick_target });
const homeChat = { host_id: null, quick_target: null };
const hostTab = (hostId: string): TabSource => ({ hostId, target: null });
const quickTab = (address: string, port = 22): TabSource => ({ hostId: null, target: { address, port, username: "root" } });

describe("conversationPlace (AI-23, AI-25)", () => {
  it("names a host, a deleted host, a quick-connect target, or none", () => {
    expect(placeName(t, conversationPlace(onHost("h-prod"), hosts))).toBe("prod-db");
    expect(placeName(t, conversationPlace(onHost("h-gone"), hosts))).toBe("Deleted host");
    expect(placeName(t, conversationPlace(onTarget("root@10.0.0.7:22"), hosts))).toBe("root@10.0.0.7:22");
    expect(placeName(t, conversationPlace(homeChat, hosts))).toBe("No host");
  });
});

describe("movedFrom (AI-09)", () => {
  it("names the host a conversation moves off, empty when it was deleted", () => {
    expect(movedFrom(onHost("h-prod"), hostTab("h-web"), hosts)).toBe("prod-db");
    expect(movedFrom(onHost("h-gone"), hostTab("h-web"), hosts)).toBe("");
    expect(movedFrom(onHost("h-prod"), quickTab("10.0.0.7"), hosts)).toBe("prod-db");
    expect(movedFrom(onHost("h-prod"), hostTab("h-prod"), hosts)).toBeNull();
  });

  it("names the quick-connect target a conversation moves off (HOST-12)", () => {
    expect(movedFrom(onTarget("root@10.0.0.7:22"), hostTab("h-prod"), hosts)).toBe("root@10.0.0.7:22");
    expect(movedFrom(onTarget("root@10.0.0.7:22"), quickTab("10.0.0.8"), hosts)).toBe("root@10.0.0.7:22");
    expect(movedFrom(onTarget("root@10.0.0.7:22"), quickTab("10.0.0.7", 2222), hosts)).toBe("root@10.0.0.7:22");
    expect(movedFrom(onTarget("root@10.0.0.7:22"), quickTab("10.0.0.7"), hosts)).toBeNull();
    // The same target written in another letter case.
    expect(movedFrom(onTarget("root@nas.local:22"), quickTab("NAS.local"), hosts)).toBeNull();
  });

  it("does not note a move between a target and the host saved from it", () => {
    expect(movedFrom(onTarget("root@10.0.0.9:22"), hostTab("h-web"), hosts)).toBeNull();
    expect(movedFrom(onHost("h-web"), quickTab("10.0.0.9"), hosts)).toBeNull();
    expect(movedFrom(onTarget("root@[2001:db8::1]:2222"), hostTab("h-v6"), hosts)).toBeNull();
    expect(movedFrom(onHost("h-v6"), quickTab("2001:db8::1", 2222), hosts)).toBeNull();
    // Addresses compare in any letter case, as quick connect compares them (HOST-12).
    expect(movedFrom(onTarget("root@nas.local:22"), hostTab("h-nas"), hosts)).toBeNull();
    expect(movedFrom(onHost("h-nas"), quickTab("nas.local"), hosts)).toBeNull();
  });

  it("does not note where a home tab chat goes", () => {
    expect(movedFrom(homeChat, hostTab("h-prod"), hosts)).toBeNull();
    expect(movedFrom(homeChat, quickTab("10.0.0.7"), hosts)).toBeNull();
  });
});

describe("connectionFor (AI-09)", () => {
  it("opens the conversation's host", () => {
    expect(connectionFor(onHost("h-prod"), hosts)).toEqual({ name: "prod-db", source: hostTab("h-prod") });
    expect(connectionFor(onHost("h-gone"), hosts)).toBeNull();
    expect(connectionFor(homeChat, hosts)).toBeNull();
  });

  it("opens its quick-connect target, as the host saved from it when there is one", () => {
    expect(connectionFor(onTarget("root@10.0.0.7:2222"), hosts)).toEqual({ name: "root@10.0.0.7:2222", source: quickTab("10.0.0.7", 2222) });
    expect(connectionFor(onTarget("root@10.0.0.9:22"), hosts)).toEqual({ name: "web-1", source: hostTab("h-web") });
    expect(connectionFor(onTarget("root@[2001:db8::1]:2222"), hosts)).toEqual({ name: "v6", source: hostTab("h-v6") });
  });
});
