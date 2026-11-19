import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TelnetHostView } from "@/ipc/types";

// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const mockApi = {
  hosts_list: vi.fn(async () => []),
  telnet_hosts_list: vi.fn<() => Promise<TelnetHostView[]>>(async () => []),
  groups_list: vi.fn(async () => []),
  keys_list: vi.fn(async () => []),
  proxies_list: vi.fn(async () => []),
  tags_list: vi.fn(async () => []),
  telnet_host_save: vi.fn(),
  telnet_host_delete: vi.fn(),
};

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: mockApi,
}));

const { t } = await import("@/i18n");
const { appError } = await import("@/ipc/api");
const { useVaultData } = await import("@/app/data");
const { useApp } = await import("@/app/store");
const { ConfirmHost, ToastHost } = await import("@hatoba/ui");
const { TelnetEditPage } = await import("./TelnetEditPage");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const TELNET_HOST: TelnetHostView = {
  id: "t-1",
  name: "core-sw",
  address: "10.0.0.2",
  port: 23,
  username: "admin",
  has_password: false,
  group_id: null,
  tags: [],
  favorite: false,
  sort: 0,
  proxy_mode: "device_default",
  proxy_id: null,
  note: "",
  ai_notes: "",
  updated_at: 1,
  last_connected_at: null,
};

let unmount: (() => void) | null = null;

function mount(hostId: string | null = TELNET_HOST.id, back: string | null = "g-1") {
  useApp.setState({ page: { kind: "telnet-edit", hostId, groupId: null, back } });
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <>
        <TelnetEditPage hostId={hostId} groupId={null} back={back} />
        <ConfirmHost />
        <ToastHost />
      </>,
    ),
  );
  unmount = () => {
    root.unmount();
    container.remove();
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  mockApi.telnet_hosts_list.mockResolvedValue([TELNET_HOST]);
  mockApi.telnet_host_save.mockResolvedValue(TELNET_HOST);
  mockApi.telnet_host_delete.mockResolvedValue(undefined);
  useVaultData.setState({ hosts: [], telnetHosts: [TELNET_HOST], groups: [], keys: [], proxies: [], tags: [] });
});

afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function setInput(id: string, value: string) {
  const input = document.getElementById(id) as HTMLInputElement;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function flush() {
  await act(async () => {
    await new Promise((r) => setTimeout(r, 0));
  });
}

const button = (label: string) =>
  [...document.querySelectorAll<HTMLButtonElement>("button")].find((b) => b.textContent === label);

describe("TelnetEditPage", () => {
  it("saves with Ctrl+S and navigates back", async () => {
    mount();
    await act(async () => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "s", ctrlKey: true, cancelable: true }));
    });
    expect(mockApi.telnet_host_save).toHaveBeenCalledTimes(1);
    expect(useApp.getState().page).toEqual({ kind: "hosts", group: "g-1" });
  });

  it("goes back on Escape when clean, and stays when changed", () => {
    mount();
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(useApp.getState().page).toEqual({ kind: "hosts", group: "g-1" });

    act(() => unmount?.());
    mount();
    setInput("telnet-name", "renamed");
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    });
    expect(useApp.getState().page).toMatchObject({ kind: "telnet-edit", hostId: TELNET_HOST.id });
  });

  it("asks for confirmation before deleting", async () => {
    mount();
    act(() => {
      button(t("hosts.delete"))!.click();
    });
    await flush();
    expect(document.querySelector("[role=alertdialog]")?.textContent).toContain(
      t("hosts.deleteTitle", { name: TELNET_HOST.name }),
    );
    expect(mockApi.telnet_host_delete).not.toHaveBeenCalled();

    await act(async () => {
      button(t("btn.delete"))!.click();
    });
    expect(mockApi.telnet_host_delete).toHaveBeenCalledWith(TELNET_HOST.id);
    expect(useApp.getState().page).toEqual({ kind: "hosts", group: "g-1" });
  });

  it("validates address and port before saving", async () => {
    mount();
    setInput("telnet-address", "   ");
    setInput("telnet-port", "0");
    await act(async () => {
      button(t("btn.save"))!.click();
    });
    expect(mockApi.telnet_host_save).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain(t("hosts.err.addressRequired"));
    expect(document.body.textContent).toContain(t("hosts.err.portInvalid"));
  });

  it("maps backend field errors and handles host/too-large and generic save errors", async () => {
    mount();
    mockApi.telnet_host_save.mockRejectedValueOnce(appError("invalid_input", "bad name", { field: "name" }));
    await act(async () => {
      button(t("btn.save"))!.click();
    });
    expect(document.body.textContent).toContain(t("err.invalid_input"));
    expect(document.activeElement?.id).toBe("telnet-name");

    mockApi.telnet_host_save.mockRejectedValueOnce(appError("invalid_input", "too large", { field: "host" }));
    await act(async () => {
      button(t("btn.save"))!.click();
    });
    expect(document.body.textContent).toContain(t("hosts.err.tooLarge"));

    mockApi.telnet_host_save.mockRejectedValueOnce(appError("io", "disk error"));
    await act(async () => {
      button(t("btn.save"))!.click();
    });
    expect(document.body.textContent).toContain(t("err.io"));
  });
});
