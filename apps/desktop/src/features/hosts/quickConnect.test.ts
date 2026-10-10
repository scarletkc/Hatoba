import { describe, expect, it } from "vitest";
import type { HostView } from "@/ipc/types";
import { formatTarget, parseQuickConnect, savedHostFor } from "./quickConnect";

const target = (address: string, port: number, username: string | null) => ({ address, port, username });

describe("parseQuickConnect", () => {
  it("leaves plain searches alone", () => {
    for (const q of ["", "prod", "prod api", "10.0.0.5", "db:5432", "ssh", "sshd", "deploy@", "@host", "prod deploy@db"])
      expect(parseQuickConnect(q), q).toBeNull();
  });

  it("reads user@host with an optional port", () => {
    expect(parseQuickConnect("deploy@43.206.11.8")).toEqual({
      ok: true,
      target: target("43.206.11.8", 22, "deploy"),
      search: "deploy@43.206.11.8",
    });
    expect(parseQuickConnect("  root@db.internal:2222 ")?.ok && parseQuickConnect("root@db.internal:2222")).toMatchObject({
      target: target("db.internal", 2222, "root"),
    });
    // The last "@" separates the user, as in ssh.
    expect(parseQuickConnect("me@corp.example@jump")).toMatchObject({ target: target("jump", 22, "me@corp.example") });
    // Still typing the port.
    expect(parseQuickConnect("root@db:")).toMatchObject({ target: target("db", 22, "root") });
  });

  it("reads ssh commands", () => {
    expect(parseQuickConnect("ssh deploy@host")).toMatchObject({ ok: true, target: target("host", 22, "deploy") });
    expect(parseQuickConnect("ssh -p 2222 deploy@host")).toMatchObject({ target: target("host", 2222, "deploy") });
    expect(parseQuickConnect("ssh -p2222 -l deploy host")).toMatchObject({ target: target("host", 2222, "deploy") });
    expect(parseQuickConnect("SSH host -l deploy -p 2200")).toMatchObject({ target: target("host", 2200, "deploy") });
    expect(parseQuickConnect("deploy@host -p 2222")).toMatchObject({ target: target("host", 2222, "deploy") });
    // -l and -p win over the destination, as in ssh.
    expect(parseQuickConnect("ssh -l alice -p 2 bob@host:3")).toMatchObject({ target: target("host", 2, "alice") });
    expect(parseQuickConnect("ssh://deploy@host:2222/")).toMatchObject({ target: target("host", 2222, "deploy") });
  });

  it("leaves the user name open when none is given", () => {
    expect(parseQuickConnect("ssh host.example.com")).toEqual({
      ok: true,
      target: target("host.example.com", 22, null),
      search: "host.example.com",
    });
  });

  it("reads IPv6 addresses", () => {
    expect(parseQuickConnect("root@[2001:db8::1]:2222")).toMatchObject({ target: target("2001:db8::1", 2222, "root") });
    expect(parseQuickConnect("root@[fe80::1%eth0]")).toMatchObject({ target: target("fe80::1%eth0", 22, "root") });
    expect(parseQuickConnect("ssh -p 2222 root@2001:db8::1")).toMatchObject({ target: target("2001:db8::1", 2222, "root") });
    expect(parseQuickConnect("ssh [::1]")).toMatchObject({ target: target("::1", 22, null) });
    expect(parseQuickConnect("root@[::1")).toBeNull();
  });

  it("explains what quick connect cannot do", () => {
    expect(parseQuickConnect("ssh -i ~/.ssh/id_ed25519 root@db")).toEqual({
      ok: false,
      problem: { kind: "option", option: "-i" },
      search: "root@db",
    });
    expect(parseQuickConnect("ssh -J jump root@db")).toMatchObject({ problem: { kind: "option", option: "-J" } });
    expect(parseQuickConnect("ssh -vA root@db")).toMatchObject({ problem: { kind: "option", option: "-v" } });
    expect(parseQuickConnect("root@db -L 8080:localhost:80")).toMatchObject({ problem: { kind: "option", option: "-L" } });
    expect(parseQuickConnect("ssh root@db uptime -p")).toMatchObject({ problem: { kind: "command" } });
    expect(parseQuickConnect("ssh -p 70000 root@db")).toMatchObject({ problem: { kind: "port" } });
    expect(parseQuickConnect("root@db:http")).toMatchObject({ problem: { kind: "port" }, search: "root@db" });
  });

  it("waits for an ssh command to be complete", () => {
    for (const q of ["ssh ", "ssh -p", "ssh -p 2222", "ssh -l deploy", "ssh -i", "ssh root@", "ssh bad!host"])
      expect(parseQuickConnect(q), q).toBeNull();
  });
});

describe("formatTarget", () => {
  it("brackets IPv6 and leaves out a missing user", () => {
    expect(formatTarget(target("db", 22, "root"))).toBe("root@db:22");
    expect(formatTarget(target("2001:db8::1", 2222, "root"))).toBe("root@[2001:db8::1]:2222");
    expect(formatTarget(target("db", 22, null))).toBe("db:22");
  });
});

describe("savedHostFor", () => {
  const host = (id: string, address: string, port: number, username: string) =>
    ({ id, address, port, username }) as HostView;
  const hosts = [host("a", "DB.internal", 22, "root"), host("b", "db.internal", 22, "deploy"), host("c", "web", 2222, "deploy")];

  it("matches address, port and user", () => {
    expect(savedHostFor(hosts, target("db.internal", 22, "deploy"))?.id).toBe("b");
    expect(savedHostFor(hosts, target("db.internal", 22, "root"))?.id).toBe("a");
    expect(savedHostFor(hosts, target("db.internal", 2222, "root"))).toBeUndefined();
    expect(savedHostFor(hosts, target("web", 2222, "root"))).toBeUndefined();
  });

  it("matches without a user only when one host fits", () => {
    expect(savedHostFor(hosts, target("web", 2222, null))?.id).toBe("c");
    expect(savedHostFor(hosts, target("db.internal", 22, null))).toBeUndefined();
  });
});
