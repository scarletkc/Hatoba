import { beforeEach, describe, expect, it, vi } from "vitest";

const skillText = vi.fn<(name: string) => Promise<string>>();
// Only the skill's text comes from the backend; this keeps the real IPC layer out of the test.
vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: { ai_skill_text: (name: string) => skillText(name) },
}));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { attachCommand } = await import("./commandActions");
const { newConversation } = await import("./actions");
const { blankSlot, getSlot, useAi } = await import("./store");

const TAB = "tab-1";

beforeEach(() => {
  useAi.setState({ slots: { [TAB]: blankSlot("manual") }, selections: {}, diagnostics: {} });
  skillText.mockReset();
});

describe("attachCommand (AI-50)", () => {
  it("attaches a skill's SKILL.md as a chip of its command", async () => {
    skillText.mockResolvedValue("# nginx");
    await expect(attachCommand(TAB, "nginx", "")).resolves.toBe("attached");
    expect(getSlot(TAB).extras.map((x) => [x.command, x.attachment.kind])).toEqual([["nginx", "skill"]]);
  });

  it("lets an unknown command go as typed, but not an MCP prompt command no running server offers", async () => {
    skillText.mockRejectedValue({ code: "not_found", detail: "skill" });
    await expect(attachCommand(TAB, "tmp", "is full")).resolves.toBe("none");
    await expect(attachCommand(TAB, "mcp__github__review_pr", "42")).resolves.toBe("failed");
    expect(getSlot(TAB).extras).toEqual([]);
  });

  it("attaches nothing once the tab shows another conversation, though New keeps the box", async () => {
    let resolve!: (text: string) => void;
    skillText.mockReturnValue(new Promise<string>((r) => (resolve = r)));
    const outcome = attachCommand(TAB, "nginx", "");
    newConversation(TAB);
    resolve("# nginx");
    await expect(outcome).resolves.toBe("failed");
    expect(getSlot(TAB).extras).toEqual([]);
  });
});
