import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { PackageManager } from "@/ipc/types";

// No Tauri here, so the pane talks to the mock backend. The app store follows the system theme
// from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { useApp } = await import("@/app/store");
const { useUpdate } = await import("@/app/update");
const { t } = await import("@/i18n");
const { AboutPane } = await import("./AboutPane");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

/** The About pane after a check found v0.2.0, in a copy that `manager` installed. */
function open(manager: PackageManager | null) {
  useApp.setState((st) => ({ info: { ...st.info, package_manager: manager } }));
  useUpdate.setState({
    result: {
      current_version: "0.1.0",
      update: { version: "0.2.0", notes: null, published_at: null, release_url: "https://github.com/scarletkc/Hatoba/releases/tag/v0.2.0" },
    },
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<AboutPane />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host;
}

const buttons = (host: HTMLElement) => [...host.querySelectorAll("button")].map((b) => b.textContent);
const autoSwitch = (host: HTMLElement) => host.querySelector(`[aria-label="${t("settings.about.auto")}"]`);

describe("AboutPane", () => {
  it("installs a newer release itself", () => {
    const host = open(null);
    expect(host.textContent).toContain(t("settings.about.available", { version: "0.2.0" }));
    expect(buttons(host)).toContain(t("settings.about.install"));
    expect(autoSwitch(host)).not.toBeNull();
  });

  it("leaves the update to Scoop, with its command to copy", () => {
    const host = open("scoop");
    expect(host.textContent).toContain(t("settings.about.available", { version: "0.2.0" }));
    expect(host.textContent).toContain(t("settings.about.manager.scoop", { command: "scoop update hatoba" }));
    expect(buttons(host)).not.toContain(t("settings.about.install"));
    expect(buttons(host)).toContain(t("settings.about.copyCommand"));
    expect(buttons(host)).toContain(t("settings.about.check"));
    expect(autoSwitch(host)).toBeNull();
  });

  it("leaves the update to the AUR helper", () => {
    const host = open("aur");
    expect(host.textContent).toContain(t("settings.about.manager.aur"));
    expect(buttons(host)).not.toContain(t("settings.about.install"));
    expect(buttons(host)).not.toContain(t("settings.about.copyCommand"));
    expect(autoSwitch(host)).toBeNull();
  });
});
