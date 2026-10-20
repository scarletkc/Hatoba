import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";

// No Tauri here, so the dialog talks to the mock backend, and choosing a file gives a made-up path.
// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { t } = await import("@/i18n");
const { api } = await import("@/ipc/api");
const { ImportDialog } = await import("./ImportDialog");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(ui: React.ReactNode) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
}

/** Lets the mock backend answer until `find` returns something. */
async function until<T>(find: () => T | null | undefined): Promise<T> {
  for (let i = 0; i < 200; i++) {
    const found = find();
    if (found) return found;
    await act(() => new Promise((r) => setTimeout(r, 10)));
  }
  throw new Error("not found in time");
}

const dialogText = () => document.querySelector("[role=dialog]")?.textContent ?? "";
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);

describe("ImportDialog", () => {
  it("lists another client's sessions with their folders and what stays behind", async () => {
    render(<ImportDialog source="xshell" onClose={() => {}} />);
    await until(() => document.querySelector("[role=dialog] [role=group]"));
    const text = dialogText();
    expect(text).toContain(t("hosts.import.titleFrom", { app: "Xshell" }));
    expect(text).toContain("mysql-master");
    expect(text).toContain(" / DB");
    expect(text).toContain(t("hosts.import.keyKept", { app: "Xshell" }));
    expect(text).toContain(t("hosts.import.proxySkipped"));
    expect(text).toContain(t("hosts.import.passwordsNote"));
    // A host that already exists is listed but not ticked.
    expect(button(t("hosts.import.submit", { n: 4 }))).toBeDefined();
  });

  it("reads the sessions again from a file the user chooses", async () => {
    render(<ImportDialog source="xshell" onClose={() => {}} />);
    await until(() => document.querySelector("[role=dialog] [role=group]"));
    expect(dialogText()).not.toContain("/demo/export.xts");
    await act(async () => button(t("hosts.import.chooseFile"))?.click());
    // Nothing is ticked, so nothing imports, until the chosen file's sessions are listed.
    expect(button(t("hosts.import.submit", { n: 0 }))?.disabled).toBe(true);
    // The mock reports the path it was given as where it read the sessions.
    await until(() => dialogText().includes(t("hosts.import.subtitle", { path: "/demo/export.xts" })) || null);
  });

  it("reads SecureCRT's sessions from an export or a configuration folder the user chooses", async () => {
    render(<ImportDialog source="securecrt" onClose={() => {}} />);
    await until(() => document.querySelector("[role=dialog] [role=group]"));
    expect(dialogText()).toContain("core-sw1");
    expect(dialogText()).toContain("HQ / Core");
    await act(async () => button(t("hosts.import.chooseFile"))?.click());
    await until(() => dialogText().includes(t("hosts.import.subtitle", { path: "/demo/export.xml" })) || null);
    await act(async () => button(t("hosts.import.chooseFolder"))?.click());
    await until(() => dialogText().includes(t("hosts.import.subtitle", { path: "/demo/folder" })) || null);
  });

  it("imports the ticked hosts into groups named after their folders, then closes", async () => {
    const onClose = vi.fn();
    render(<ImportDialog source="tabby" onClose={onClose} />);
    await until(() => document.querySelector("[role=dialog] [role=group]"));
    await act(async () => button(t("hosts.import.submit", { n: 3 }))?.click());
    await until(() => onClose.mock.calls.length > 0 || null);
    const hosts = await api.hosts_list();
    const api01 = hosts.find((h) => h.name === "api");
    const groups = await api.groups_list();
    expect(groups.find((g) => g.id === api01?.group_id)?.name).toBe("Work / API");
  });

  it("names ~/.ssh/config and has nothing to choose for it", async () => {
    render(<ImportDialog source="ssh_config" onClose={() => {}} />);
    await until(() => document.querySelector("[role=dialog] [role=group]"));
    expect(dialogText()).toContain(t("hosts.import.title"));
    expect(dialogText()).toContain(t("hosts.import.proxyCommand"));
    expect(dialogText()).not.toContain(t("hosts.import.passwordsNote"));
    expect(button(t("hosts.import.chooseFile"))).toBeUndefined();
    expect(button(t("hosts.import.chooseFolder"))).toBeUndefined();
  });
});
