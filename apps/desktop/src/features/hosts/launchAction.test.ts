import { describe, expect, it } from "vitest";
import type { HostView, TelnetHostView } from "@/ipc/types";
import { launchAction } from "./launchAction";

const host = (id: string, name: string, address: string, port = 22, username = "root") =>
  ({ id, name, address, port, username }) as HostView;
const hosts = [
  host("prod", "Prod Web", "10.0.0.5"),
  host("db", "db", "db.internal", 22, "postgres"),
  host("twin-a", "twin", "10.0.0.7"),
  host("twin-b", "Twin", "10.0.0.8"),
];
const telnetHosts = [{ id: "core", name: "Core Switch", address: "10.0.0.2", port: 23 }] as TelnetHostView[];
const launch = (args: string[]) => launchAction(args, hosts, telnetHosts);
const target = (address: string, port: number, username: string | null) => ({
  kind: "target",
  target: { address, port, username },
});
const telnet = (address: string, port: number) => ({ kind: "telnet", target: { address, port } });

describe("launchAction", () => {
  it("only brings the window up without arguments", () => {
    expect(launch([])).toBeNull();
    expect(launch(["  "])).toBeNull();
  });

  it("connects a saved host by its name, in any case and split into words", () => {
    expect(launch(["prod", "web"])).toEqual({ kind: "host", hostId: "prod" });
    expect(launch(["PROD WEB"])).toEqual({ kind: "host", hostId: "prod" });
    expect(launch(["db"])).toEqual({ kind: "host", hostId: "db" });
    expect(launch(["core", "switch"])).toEqual({ kind: "host", hostId: "core" });
  });

  it("searches a name that several hosts have", () => {
    expect(launch(["twin"])).toEqual({ kind: "search", text: "twin" });
    const dbConsole = [{ id: "db-console", name: "DB", address: "10.0.0.9", port: 23 }] as TelnetHostView[];
    expect(launchAction(["db"], hosts, dbConsole)).toEqual({ kind: "search", text: "db" });
  });

  it("reads ssh arguments, with or without the word", () => {
    expect(launch(["deploy@web.example.com"])).toEqual(target("web.example.com", 22, "deploy"));
    expect(launch(["-p", "2222", "deploy@web"])).toEqual(target("web", 2222, "deploy"));
    expect(launch(["ssh", "-l", "deploy", "web"])).toEqual(target("web", 22, "deploy"));
    expect(launch(["ssh://deploy@[2001:db8::1]:2200"])).toEqual(target("2001:db8::1", 2200, "deploy"));
    // Like `ssh web`: the local user, on port 22.
    expect(launch(["web.example.com"])).toEqual(target("web.example.com", 22, null));
  });

  it("reads telnet arguments (TELNET-05)", () => {
    expect(launch(["telnet", "10.0.0.3"])).toEqual(telnet("10.0.0.3", 23));
    expect(launch(["telnet", "10.0.0.3", "2323"])).toEqual(telnet("10.0.0.3", 2323));
    expect(launch(["telnet://10.0.0.3"])).toEqual(telnet("10.0.0.3", 23));
    expect(launch(["TELNET://[2001:db8::1]:2323"])).toEqual(telnet("2001:db8::1", 2323));
  });

  it("connects the saved host a target matches", () => {
    expect(launch(["root@10.0.0.5"])).toEqual({ kind: "host", hostId: "prod" });
    expect(launch(["db.internal"])).toEqual({ kind: "host", hostId: "db" });
    expect(launch(["telnet", "10.0.0.2"])).toEqual({ kind: "host", hostId: "core" });
  });

  it("shows why ssh and telnet syntax cannot connect, and searches other words", () => {
    expect(launch(["root@db", "uptime"])).toEqual({ kind: "search", text: "ssh root@db uptime" });
    expect(launch(["-L", "8080:localhost:80", "web"])).toEqual({
      kind: "search",
      text: "ssh -L 8080:localhost:80 web",
    });
    expect(launch(["ssh", "-p", "99999", "web"])).toEqual({ kind: "search", text: "ssh -p 99999 web" });
    expect(launch(["telnet", "10.0.0.3", "99999"])).toEqual({ kind: "search", text: "telnet 10.0.0.3 99999" });
    expect(launch(["prod", "api"])).toEqual({ kind: "search", text: "prod api" });
    expect(launch(["ssh"])).toEqual({ kind: "search", text: "ssh" });
    expect(launch(["telnet"])).toEqual({ kind: "search", text: "telnet" });
  });
});
