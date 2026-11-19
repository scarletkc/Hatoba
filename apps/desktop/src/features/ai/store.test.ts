import { describe, expect, it, vi } from "vitest";
import { readStoredPrefs } from "@/ipc/mock/prefs";

// The slot test reads no command; this keeps the real IPC layer out of it.
vi.mock("@/ipc/api", async (importActual) => ({ ...(await importActual<typeof import("@/ipc/api")>()), api: {} }));
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { blankSlot } = await import("./store");

describe("blankSlot", () => {
  it("starts new conversations in a mode this build knows", () => {
    useApp.setState({ prefs: readStoredPrefs(() => JSON.stringify({ ai_permission_mode: "ask_twice" })) });
    expect(blankSlot().mode).toBe("manual");
    useApp.setState({ prefs: readStoredPrefs(() => JSON.stringify({ ai_permission_mode: "bypass" })) });
    expect(blankSlot().mode).toBe("bypass");
  });
});
