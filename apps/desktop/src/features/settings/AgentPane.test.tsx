import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

// No Tauri here, so the pane talks to the mock backend. The app store follows the system theme
// from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { t } = await import("@/i18n");
const { api } = await import("@/ipc/api");
const { AgentPane } = await import("./AgentPane");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  act(() => useApp.setState({ prefs: { ...useApp.getState().prefs, ssh_agent_socket: null } }));
});

function render() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<AgentPane />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

/** Lets the mock backend answer until `find` returns something. */
async function until<T>(find: () => T | null | undefined | false | Promise<T | false>): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const found = await find();
    if (found) return found;
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  throw new Error("not found in time");
}

const byLabel = <E extends HTMLElement>(label: string) => document.querySelector<E>(`[aria-label="${label}"]`);
const body = () => document.body.textContent ?? "";

/** Opens the menu of a popup button and picks the option labelled `label`. */
function pick(button: HTMLButtonElement, label: string) {
  act(() => button.click());
  const option = [...document.querySelectorAll<HTMLButtonElement>("[role=menu] button")].find((b) => b.textContent === label);
  if (!option) throw new Error(`no option ${label}`);
  act(() => option.click());
}

const agentButton = () => byLabel<HTMLButtonElement>(t("settings.agent.source"));

describe("AgentPane", () => {
  it("shows the agent that answered with the keys it holds", async () => {
    render();
    const button = await until(agentButton);

    // The device's agent, not a choice of the user's: this build finds it by its socket.
    expect(button.textContent).toContain(t("settings.agent.automatic"));
    // The socket the agent answered on, which only a finished read shows: "Bitwarden" alone is
    // also in the automatic choice's hint.
    const socket = "/Users/you/.bitwarden-ssh-agent.sock";
    await until(() => (body().includes(socket) ? true : null));
    expect(body()).toContain(t("settings.agent.kind.bitwarden"));
    expect(body()).toContain(t("settings.agent.keys", { n: 2 }));
    expect(body()).toContain("SHA256:BIv2JWQvx1+aWk8MNJ02qMyFwUbgq8sipBVydzw2mmo");
  });

  it("offers only the automatic choice and a path of the user's own", async () => {
    render();
    const button = await until(agentButton);
    act(() => button.click());
    const rows = [...document.querySelectorAll<HTMLButtonElement>("[role=menu] button")].map((b) => b.textContent);
    expect(rows).toEqual([t("settings.agent.automatic"), t("settings.agent.custom")]);
  });

  it("keeps a socket the user types and reports what the backend answers for it", async () => {
    render();
    const button = await until(agentButton);

    pick(button, t("settings.agent.custom"));
    const field = await until(() => byLabel<HTMLInputElement>(t("settings.agent.path")));
    // Choosing a custom path changes nothing until one is named.
    expect(useApp.getState().prefs.ssh_agent_socket).toBe(null);
    expect(field.value).toBe("");

    act(() => {
      const set = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(field), "value")?.set;
      set?.call(field, "/tmp/hatoba-agent.sock");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(() => {
      field.dispatchEvent(new FocusEvent("focusout", { bubbles: true }));
    });

    await until(() => (useApp.getState().prefs.ssh_agent_socket === "/tmp/hatoba-agent.sock" ? true : null));
    // The path is kept over a reload, and the tab shows why the agent behind it does not answer.
    expect((await api.prefs_get()).ssh_agent_socket).toBe("/tmp/hatoba-agent.sock");
    await until(() => (body().includes("/tmp/hatoba-agent.sock") ? true : null));
    expect(body()).toContain(t("settings.agent.hint"));
    expect((await until(() => byLabel<HTMLInputElement>(t("settings.agent.path")))).value).toBe("/tmp/hatoba-agent.sock");
  });
});
