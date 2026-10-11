import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setLocale, t, type Locale } from "@/i18n";
import type { HostKeyPrompt } from "@/ipc/types";
import { pushHostKeyPrompt, usePrompts } from "./prompts";
import { SessionDialogs } from "./SessionDialogs";

vi.mock("./listeners", () => ({ startSessionListeners: () => () => {} }));
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  usePrompts.setState({ items: [] });
});

function show(prompt: Partial<HostKeyPrompt>, locale: Locale = "en") {
  setLocale(locale);
  pushHostKeyPrompt({
    request_id: "hostkey-test",
    session_id: "session-test",
    host: "demo.example",
    port: 22,
    key_type: "ssh-ed25519",
    fingerprint: "SHA256:received-demo-key",
    kind: "new",
    known_fingerprint: null,
    known_key_type: null,
    ...prompt,
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<SessionDialogs />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return document.querySelector<HTMLElement>("[role=alertdialog]")!;
}

describe("host key dialogs", () => {
  it("keeps the first-connection prompt for an unknown host", () => {
    const dialog = show({});
    expect(dialog.textContent).toContain(t("terminal.hostkey.newBody"));
    expect(dialog.textContent).toContain("SHA256:received-demo-key");
  });

  it.each(["en", "zh-CN", "ja"] as const)("shows the other trusted key when asking to trust a new key type in %s", (locale) => {
    const dialog = show({ known_key_type: "ssh-rsa", known_fingerprint: "SHA256:trusted-demo-rsa" }, locale);
    expect(dialog.textContent).toContain(t("terminal.hostkey.additionalTitle", { host: "demo.example", port: 22 }));
    expect(dialog.textContent).toContain(t("terminal.hostkey.additionalBody"));
    expect(document.activeElement?.textContent).toBe(t("btn.cancel"));
    expect(dialog.textContent).toContain("ssh-rsa");
    expect(dialog.textContent).toContain("SHA256:trusted-demo-rsa");
    expect(dialog.textContent).toContain("ssh-ed25519");
    expect(dialog.textContent).not.toContain(t("terminal.hostkey.newBody"));
    expect(dialog.textContent).not.toContain(t("terminal.hostkey.changedTitle"));
  });

  it("keeps the changed-key warning and focuses Disconnect", () => {
    const dialog = show({ kind: "changed", known_key_type: "ssh-ed25519", known_fingerprint: "SHA256:trusted-demo-key" });
    expect(dialog.textContent).toContain(t("terminal.hostkey.changedTitle"));
    expect(dialog.textContent).toContain("SHA256:trusted-demo-key");
    expect(document.activeElement?.textContent).toBe(t("terminal.hostkey.abort"));
  });
});
