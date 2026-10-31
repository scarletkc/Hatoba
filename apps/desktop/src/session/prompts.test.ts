import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AuthBanner, AuthPrompt, HostKeyPrompt } from "@/ipc/types";

const answered = vi.hoisted(() => ({ hostkeys: [] as [string, boolean][], auths: [] as [string, string[] | null][] }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    hostkey_respond: async (id: string, accept: boolean) => void answered.hostkeys.push([id, accept]),
    auth_prompt_respond: async (id: string, answers: string[] | null) => void answered.auths.push([id, answers]),
  },
}));

const { cancelSessionPrompts, closeAuthPrompt, pushAuthBanner, pushAuthPrompt, pushHostKeyPrompt, usePrompts } =
  await import("./prompts");

const hostKey = (request_id: string, session_id: string): HostKeyPrompt => ({
  request_id,
  session_id,
  host: "db",
  port: 22,
  key_type: "ssh-ed25519",
  fingerprint: "SHA256:x",
  kind: "new",
  known_fingerprint: null,
  known_key_type: null,
});

const login = (request_id: string, session_id: string, password: boolean): AuthPrompt => ({
  request_id,
  session_id,
  name: "db",
  instructions: "",
  prompts: [{ prompt: "Password: ", echo: false }],
  password,
  sudo: false,
  wrong: false,
  target: "root@db:22",
});

describe("pushAuthPrompt", () => {
  it("asks for a sudo password as a secret of the saved host, and answers with it", () => {
    usePrompts.setState({ items: [] });
    answered.auths.length = 0;
    pushAuthPrompt({ ...login("sudo1", "", false), session_id: null, name: "web", sudo: true, wrong: true });
    const [item] = usePrompts.getState().items;
    expect(item.kind === "secret" && item.request).toEqual({
      kind: "sudo",
      hostName: "web",
      target: "root@db:22",
      wrong: true,
    });
    if (item.kind === "secret") item.resolve("pw");
    expect(answered.auths).toEqual([["sudo1", ["pw"]]]);
    usePrompts.setState({ items: [] });
    answered.auths.length = 0;
  });
});

describe("closeAuthPrompt", () => {
  it("drops the prompt no longer waited for, unanswered, and leaves the others", () => {
    pushAuthPrompt({ ...login("sudo1", "", false), session_id: null, sudo: true });
    pushAuthPrompt(login("ki1", "s1", false));
    pushAuthPrompt(login("pw1", "s1", true));
    pushAuthPrompt({ ...login("sudo2", "", false), session_id: null, sudo: true });

    closeAuthPrompt({ request_id: "sudo1" });
    closeAuthPrompt({ request_id: "ki1" });
    closeAuthPrompt({ request_id: "pw1" });

    expect(answered.auths).toEqual([]);
    const left = usePrompts.getState().items;
    expect(left).toHaveLength(1);
    expect(left[0].kind === "secret" && left[0].requestId).toBe("sudo2");
    usePrompts.setState({ items: [] });
  });
});

describe("cancelSessionPrompts", () => {
  it("answers the prompts of a closed tab's connection and leaves the others", () => {
    pushHostKeyPrompt(hostKey("hk1", "s1"));
    pushAuthPrompt(login("pw1", "s1", true));
    pushAuthPrompt(login("ki1", "s1", false));
    pushAuthPrompt(login("pw2", "s2", true));

    cancelSessionPrompts("s1");

    expect(answered.hostkeys).toEqual([["hk1", false]]);
    expect(answered.auths).toEqual([
      ["pw1", null],
      ["ki1", null],
    ]);
    const left = usePrompts.getState().items;
    expect(left).toHaveLength(1);
    expect(left[0].kind === "secret" && left[0].sessionId).toBe("s2");
  });
});

const banner = (request_id: string, session_id: string | null): AuthBanner => ({
  request_id,
  session_id,
  target: "alice@nas.example.ts.net:22",
  text: "# To authenticate, visit: https://login.tailscale.com/a/1234",
});

describe("pushAuthBanner", () => {
  beforeEach(() => {
    usePrompts.setState({ items: [] });
    answered.auths.length = 0;
  });
  afterEach(() => vi.useRealTimers());

  it("shows a banner once the server has held the login for a second, until it answers", () => {
    vi.useFakeTimers();
    pushAuthBanner(banner("b1", null));
    vi.advanceTimersByTime(999);
    expect(usePrompts.getState().items).toEqual([]);
    vi.advanceTimersByTime(1);
    const [item] = usePrompts.getState().items;
    expect(item.kind === "banner" && item.banner.request_id).toBe("b1");

    closeAuthPrompt({ request_id: "b1" });
    expect(usePrompts.getState().items).toEqual([]);
    expect(answered.auths).toEqual([]);
  });

  it("never shows a banner the server answers at once", () => {
    vi.useFakeTimers();
    pushAuthBanner(banner("b1", "s1"));
    closeAuthPrompt({ request_id: "b1" });
    vi.advanceTimersByTime(5000);
    expect(usePrompts.getState().items).toEqual([]);
  });

  it("stops waiting on the banners of a closed tab's connection, shown or not", () => {
    vi.useFakeTimers();
    pushAuthBanner(banner("b1", "s1"));
    vi.advanceTimersByTime(1000);
    pushAuthBanner(banner("b2", "s1"));
    pushAuthBanner(banner("b3", "s2"));

    cancelSessionPrompts("s1");
    vi.advanceTimersByTime(1000);

    expect(answered.auths).toEqual([
      ["b2", null],
      ["b1", null],
    ]);
    const left = usePrompts.getState().items;
    expect(left).toHaveLength(1);
    expect(left[0].kind === "banner" && left[0].banner.request_id).toBe("b3");
  });
});
