import { afterEach, describe, expect, it } from "vitest";
import { commands } from "../bindings";
import type { EventMap } from "../types";
import { createMockApi } from "./index";

const snakeCase = (name: string) => name.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

afterEach(() => {
  localStorage.clear();
  history.replaceState(null, "", "/");
});

describe("createMockApi", () => {
  it("implements every command from bindings.ts plus listen", () => {
    const mock = createMockApi();
    for (const name of Object.keys(commands)) {
      const key = snakeCase(name);
      expect(typeof (mock as unknown as Record<string, unknown>)[key], key).toBe("function");
    }
    expect(typeof mock.listen).toBe("function");
  });

  it("handles vault, PIN, and biometric unlock flows", async () => {
    const mock = createMockApi();
    expect((await mock.vault_status()).state).toBe("unlocked");

    await mock.vault_lock();
    expect((await mock.vault_status()).state).toBe("locked");
    await expect(mock.hosts_list()).rejects.toMatchObject({ code: "locked" });

    await expect(mock.vault_unlock("no")).rejects.toMatchObject({ code: "wrong_password" });
    await mock.vault_unlock("hatoba");
    expect((await mock.vault_status()).state).toBe("unlocked");

    await expect(mock.pin_enable("hatoba", "12")).rejects.toMatchObject({ code: "invalid_input", field: "pin" });
    await mock.pin_enable("hatoba", "2468");
    await mock.pin_set_persist(true);
    const status = await mock.vault_status();
    expect(status.pin_enabled).toBe(true);
    expect(status.pin_persist).toBe(true);
    expect(status.pin_unlock).toBe(true);

    await mock.vault_lock();
    await expect(mock.vault_unlock_pin("0000")).rejects.toMatchObject({ code: "wrong_pin", attempts_left: 4 });
    await mock.vault_unlock_pin("2468");
    expect((await mock.vault_status()).state).toBe("unlocked");

    await mock.pin_disable();
    expect((await mock.vault_status()).pin_enabled).toBe(false);

    await mock.biometric_enable("hatoba");
    expect((await mock.vault_status()).biometric_enabled).toBe(true);
    await mock.vault_lock();
    await mock.vault_unlock_biometric();
    expect((await mock.vault_status()).state).toBe("unlocked");
    await mock.biometric_disable();
    expect((await mock.vault_status()).biometric_enabled).toBe(false);

    const recovery = await mock.vault_create("correct-horse-battery");
    expect(recovery).toBeTruthy();
    expect(await mock.hosts_list()).toEqual([]);
  }, 10_000);

  it("handles telnet host CRUD and session open/write/close", async () => {
    const mock = createMockApi();
    const events: EventMap["ssh://state"][] = [];
    const unlisten = await mock.listen("ssh://state", (e) => events.push(e));

    const initial = await mock.telnet_hosts_list();
    expect(initial.length).toBeGreaterThan(0);

    await expect(
      mock.telnet_host_save({
        id: null,
        name: "Bad",
        address: "   ",
        port: 23,
        username: "",
        password: null,
        group_id: null,
        tags: [],
        favorite: false,
        proxy_mode: "device_default",
        proxy_id: null,
        note: "",
        ai_notes: "",
      }),
    ).rejects.toMatchObject({ code: "invalid_input", field: "address" });

    const created = await mock.telnet_host_save({
      id: null,
      name: "Core Switch",
      address: "10.1.2.3",
      port: 23,
      username: "admin",
      password: "secret",
      group_id: null,
      tags: ["net"],
      favorite: false,
      proxy_mode: "device_default",
      proxy_id: null,
      note: "",
      ai_notes: "",
    });
    expect(created.has_password).toBe(true);

    await mock.telnet_host_set_favorite(created.id, true);
    const copy = await mock.telnet_host_duplicate(created.id);
    expect(copy.name).toBe("Core Switch-copy");
    expect(copy.favorite).toBe(false);

    const frames: Uint8Array[] = [];
    const sid = await mock.telnet_open(created.id, { cols: 80, rows: 24 }, (f) => frames.push(f));
    await mock.telnet_write(sid, "help\r");
    await mock.telnet_close(sid);
    await new Promise((r) => setTimeout(r, 20));
    expect(events.some((e) => e.session_id === sid && e.state === "connected")).toBe(true);

    await mock.telnet_host_delete(copy.id);
    await mock.telnet_host_delete(created.id);
    expect((await mock.telnet_hosts_list()).some((h) => h.id === created.id)).toBe(false);
    unlisten();
  });

  it("handles keys and proxies CRUD and host cleanup on delete", async () => {
    const mock = createMockApi();
    await expect(
      mock.key_import({
        name: "bad",
        path: null,
        private_key: "not a key",
        passphrase: null,
      }),
    ).rejects.toMatchObject({ code: "key_parse", key_kind: "unsupported_format" });

    const generated = await mock.key_generate({
      name: "ed-test",
      algorithm: "ed25519",
      comment: "dev@hatoba",
      passphrase: null,
    });
    expect(await mock.key_public(generated.id)).toContain("ssh-ed25519");
    const renamed = await mock.key_rename(generated.id, "ed-renamed");
    expect(renamed.name).toBe("ed-renamed");

    const proxy = await mock.proxy_save({
      id: null,
      name: "Corp SOCKS",
      kind: "socks5",
      address: "127.0.0.1",
      port: 1080,
      username: "",
      password: null,
    });
    const host = await mock.host_save({
      id: null,
      name: "Proxy Host",
      address: "10.2.3.4",
      port: 22,
      username: "deploy",
      auth_kind: "key",
      password: null,
      key_id: generated.id,
      group_id: null,
      tags: [],
      favorite: false,
      jump_host_id: null,
      proxy_mode: "proxy",
      proxy_id: proxy.id,
      note: "",
      ai_notes: "",
      env: [],
    });

    const proxies = await mock.proxies_list();
    expect(proxies.find((p) => p.id === proxy.id)?.host_ids).toContain(host.id);

    await mock.key_delete(generated.id);
    await mock.proxy_delete(proxy.id);
    const updatedHost = await mock.host_get(host.id);
    expect(updatedHost.key_id).toBeNull();
    expect(updatedHost.auth_kind).toBe("ask");
    expect(updatedHost.proxy_mode).toBe("device_default");
    expect(updatedHost.proxy_id).toBeNull();
  });

  it("handles sync configuration and disconnect flows", async () => {
    const mock = createMockApi();
    const statuses: EventMap["sync://status"][] = [];
    const unlisten = await mock.listen("sync://status", (s) => statuses.push(s));

    const testOk = await mock.sync_test({ kind: "worker", url: "https://sync.example.workers.dev", setup_token: null });
    expect(testOk.ok).toBe(true);

    await mock.sync_configure({ kind: "worker", url: "https://sync.example.workers.dev", setup_token: null }, "hatoba");
    expect((await mock.sync_status()).endpoint).toBe("sync.example.workers.dev");
    expect(statuses.length).toBeGreaterThan(0);

    const conflicts = await mock.sync_conflicts();
    if (conflicts.length > 0) {
      await mock.sync_conflict_resolve(conflicts[0].id, "keep");
    }

    await mock.sync_disconnect();
    expect((await mock.sync_status()).kind).toBe("none");
    unlisten();
  });

  it("handles deploy start, inspect, run, and setup flows", async () => {
    const mock = createMockApi();
    const started = await mock.deploy_start("a-valid-cloudflare-token-20", null);
    const inspected = await mock.deploy_inspect(started.handle, {
      account_id: started.accounts[0].id,
      worker_name: "hatoba-sync",
      database_name: "hatoba",
      subdomain: "kc",
    });
    expect(inspected.subdomain).toBe("kc");

    const steps: string[] = [];
    const deployed = await mock.deploy_run(
      started.handle,
      {
        account_id: started.accounts[0].id,
        worker_name: "hatoba-sync",
        database_name: "hatoba",
        subdomain: "kc",
      },
      (p) => steps.push(`${p.step}:${p.status}`),
    );
    expect(deployed.ready).toBe(true);
    expect(steps.length).toBeGreaterThan(0);

    await mock.deploy_setup(started.handle, "hatoba");
    expect((await mock.sync_status()).endpoint).toBe("hatoba-sync.kc.workers.dev");
  }, 15_000);
});
