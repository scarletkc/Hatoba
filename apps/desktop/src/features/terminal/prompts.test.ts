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

const { cancelSessionPrompts, pushAuthPrompt, pushHostKeyPrompt, usePrompts } = await import("./prompts");

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
  target: "root@db:22",
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
