import { describe, expect, it, vi } from "vitest";
import type { AuthPrompt, HostKeyPrompt } from "@/ipc/types";

const answered = vi.hoisted(() => ({ hostkeys: [] as [string, boolean][], auths: [] as [string, string[] | null][] }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    hostkey_respond: async (id: string, accept: boolean) => void answered.hostkeys.push([id, accept]),
    auth_prompt_respond: async (id: string, answers: string[] | null) => void answered.auths.push([id, answers]),
  },
}));

const { cancelSessionPrompts, closeAuthPrompt, pushAuthPrompt, pushHostKeyPrompt, usePrompts } =
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
    pushAuthPrompt({ ...login("sudo2", "", false), session_id: null, sudo: true });

    closeAuthPrompt({ request_id: "sudo1" });
    closeAuthPrompt({ request_id: "ki1" });

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
