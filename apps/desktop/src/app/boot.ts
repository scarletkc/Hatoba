import { api } from "@/ipc/api";
import { applyPrefs, DEFAULT_PREFS, useApp } from "./store";
import { useVaultData } from "./data";

/** Load platform info, device-local prefs and vault state before the first render. */
export async function boot() {
  const root = document.documentElement;
  try {
    const info = await api.app_info();
    root.dataset.platform = info.platform;
    if (info.mica) root.dataset.material = "mica";
    useApp.setState({ info });
    if (info.platform === "linux") useApp.setState({ floating: await api.window_floating().catch(() => false) });

    const prefs = { ...DEFAULT_PREFS, ...(await api.prefs_get()) };
    useApp.setState({ prefs });
    applyPrefs(prefs);

    const vault = await useApp.getState().refreshVault();
    if (vault.state === "unlocked") await enterUnlocked();
    else useApp.setState({ phase: vault.state === "uninitialized" ? "onboarding" : "locked" });
  } catch (e) {
    console.error("boot failed", e);
    applyPrefs(DEFAULT_PREFS);
    useApp.setState({ phase: "locked" });
  }
}

/** Called after create / unlock / restore: load vault data and sync status, then show the main UI. */
export async function enterUnlocked() {
  await useVaultData.getState().reload();
  try {
    useApp.getState().setSync(await api.sync_status());
  } catch {
    /* sync status is optional */
  }
  void useApp.getState().refreshVault();
  useApp.setState({ phase: "unlocked" });
}
