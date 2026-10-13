import { describe, expect, it } from "vitest";
import type { HostView } from "@/ipc/types";
import { launchAction } from "./launchAction";

const host = (id: string, name: string, address: string, port = 22, username = "root") =>
  ({ id, name, address, port, username }) as HostView;
const hosts = [
  host("prod", "Prod Web", "10.0.0.5"),
  host("db", "db", "db.internal", 22, "postgres"),
  host("twin-a", "twin", "10.0.0.7"),
  host("twin-b", "Twin", "10.0.0.8"),
];
const target = (address: string, port: number, username: string | null) => ({ kind: "target", target: { address, port, username } });

describe("launchAction", () => {
  it("only brings the window up without arguments", () => {
    expect(launchAction([], hosts)).toBeNull();
    expect(launchAction(["  "], hosts)).toBeNull();
  });

  it("connects a saved host by its name, in any case and split into words", () => {
    expect(launchAction(["prod", "web"], hosts)).toEqual({ kind: "host", hostId: "prod" });
    expect(launchAction(["PROD WEB"], hosts)).toEqual({ kind: "host", hostId: "prod" });
    expect(launchAction(["db"], hosts)).toEqual({ kind: "host", hostId: "db" });
  });

  it("searches a name that several hosts have", () => {
    expect(launchAction(["twin"], hosts)).toEqual({ kind: "search", text: "twin" });
  });

  it("reads ssh arguments, with or without the word", () => {
    expect(launchAction(["deploy@web.example.com"], hosts)).toEqual(target("web.example.com", 22, "deploy"));
    expect(launchAction(["-p", "2222", "deploy@web"], hosts)).toEqual(target("web", 2222, "deploy"));
    expect(launchAction(["ssh", "-l", "deploy", "web"], hosts)).toEqual(target("web", 22, "deploy"));
    expect(launchAction(["ssh://deploy@[2001:db8::1]:2200"], hosts)).toEqual(target("2001:db8::1", 2200, "deploy"));
    // Like `ssh web`: the local user, on port 22.
    expect(launchAction(["web.example.com"], hosts)).toEqual(target("web.example.com", 22, null));
  });

  it("connects the saved host a target matches", () => {
    expect(launchAction(["root@10.0.0.5"], hosts)).toEqual({ kind: "host", hostId: "prod" });
    expect(launchAction(["db.internal"], hosts)).toEqual({ kind: "host", hostId: "db" });
  });

  it("shows why ssh syntax cannot connect, and searches other words", () => {
    expect(launchAction(["root@db", "uptime"], hosts)).toEqual({ kind: "search", text: "ssh root@db uptime" });
    expect(launchAction(["-L", "8080:localhost:80", "web"], hosts)).toEqual({ kind: "search", text: "ssh -L 8080:localhost:80 web" });
    expect(launchAction(["ssh", "-p", "99999", "web"], hosts)).toEqual({ kind: "search", text: "ssh -p 99999 web" });
    expect(launchAction(["prod", "api"], hosts)).toEqual({ kind: "search", text: "prod api" });
    expect(launchAction(["ssh"], hosts)).toEqual({ kind: "search", text: "ssh" });
  });
});
