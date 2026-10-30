import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { RECOVERY_AUTH, call, envelope, login, setupVault } from "./helpers";

describe("GET /v1/devices", () => {
  it("lists devices, flags the caller and keeps device_name opaque", async () => {
    await setupVault();
    const a = await login("device-a", undefined, "name-envelope-a");
    await login("device-b", undefined, "name-envelope-b");

    const res = await call("/v1/devices", { token: a.session_token });
    expect(res.status).toBe(200);
    const devices = res.body.devices as Array<Record<string, unknown>>;
    expect(devices).toHaveLength(2);

    const byId = Object.fromEntries(devices.map((d) => [d.device_id as string, d]));
    expect(byId["device-a"]).toMatchObject({ device_name: "name-envelope-a", current: true });
    expect(byId["device-b"]).toMatchObject({ device_name: "name-envelope-b", current: false });
    for (const device of devices) {
      expect(Object.keys(device).sort()).toEqual(
        ["created_at", "current", "device_id", "device_name", "expires_at", "last_seen"].sort(),
      );
      expect(device.expires_at as number).toBeGreaterThan(Date.now());
    }
    expect(JSON.stringify(res.body)).not.toContain("token");
  });

  it("hides expired sessions and restricted recovery sessions", async () => {
    await setupVault();
    const a = await login("device-a");
    await login("device-old");
    await call("/v1/recover", {
      json: { recovery_auth: RECOVERY_AUTH, device_id: "device-r", device_name: envelope() },
    });
    await env.DB.prepare("UPDATE sessions SET expires_at = ? WHERE device_id = 'device-old'")
      .bind(Date.now() - 1)
      .run();

    const res = await call("/v1/devices", { token: a.session_token });
    expect(res.body.devices.map((d: { device_id: string }) => d.device_id)).toEqual(["device-a"]);
  });

  it("orders devices by most recent activity", async () => {
    await setupVault();
    const a = await login("device-a");
    await login("device-b");
    await env.DB.prepare("UPDATE sessions SET last_seen = ? WHERE device_id = 'device-b'")
      .bind(Date.now() + 5000)
      .run();
    const res = await call("/v1/devices", { token: a.session_token });
    expect(res.body.devices.map((d: { device_id: string }) => d.device_id)).toEqual(["device-b", "device-a"]);
  });
});

describe("DELETE /v1/devices/:device_id", () => {
  it("revokes another device: 204, its token stops working, the caller is unaffected", async () => {
    await setupVault();
    const a = await login("device-a");
    const b = await login("device-b");

    const res = await call("/v1/devices/device-b", { method: "DELETE", token: a.session_token });
    expect(res.status).toBe(204);
    expect(res.body).toBe("");

    expect((await call("/v1/vault", { token: b.session_token })).status).toBe(401);
    expect((await call("/v1/vault", { token: a.session_token })).status).toBe(200);
    const list = await call("/v1/devices", { token: a.session_token });
    expect(list.body.devices.map((d: { device_id: string }) => d.device_id)).toEqual(["device-a"]);
  });

  it("allows a device to revoke itself", async () => {
    await setupVault();
    const a = await login("device-a");
    const res = await call("/v1/devices/device-a", { method: "DELETE", token: a.session_token });
    expect(res.status).toBe(204);
    expect((await call("/v1/vault", { token: a.session_token })).status).toBe(401);
  });

  it("is idempotent for unknown devices and validates the id", async () => {
    await setupVault();
    const a = await login("device-a");
    expect((await call("/v1/devices/never-existed", { method: "DELETE", token: a.session_token })).status).toBe(204);
    const bad = await call("/v1/devices/bad%20id", { method: "DELETE", token: a.session_token });
    expect(bad.status).toBe(400);
  });

  it("lets a revoked device log in again", async () => {
    await setupVault();
    const a = await login("device-a");
    await call("/v1/devices/device-b", { method: "DELETE", token: a.session_token });
    expect((await login("device-b")).session_token).toBeTruthy();
  });
});
