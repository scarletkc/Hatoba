import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { errorMessage } from "@/app/errors";
import { useApp, type SettingsTab } from "@/app/store";
import { Icon, IconButton } from "@/components/controls";
import { Modal, toast } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import { api } from "@/ipc/api";
import type { SettingsView, TerminalSettings } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { AboutPane } from "./AboutPane";
import { AppearancePane } from "./AppearancePane";
import { GeneralPane } from "./GeneralPane";
import { SecurityPane } from "./SecurityPane";
import { TerminalPane } from "./TerminalPane";
import s from "./SettingsWindow.module.css";

const TABS: { id: SettingsTab; icon: string; label: MessageKey }[] = [
  { id: "general", icon: "gear-six", label: "settings.tab.general" },
  { id: "appearance", icon: "paint-brush", label: "settings.tab.appearance" },
  { id: "terminal", icon: "terminal-window", label: "settings.tab.terminal" },
  { id: "security", icon: "lock-key", label: "settings.tab.security" },
  { id: "about", icon: "info", label: "settings.tab.about" },
];

let lastTab: SettingsTab = "general";

/**
 * Settings (design §07): a 700×620 preferences window with General / Appearance / Terminal / Security, plus
 * About for the version and the update check.
 */
export function SettingsWindow({ onClose }: { onClose: () => void }) {
  const t = useT();
  const [tab, setTabState] = useState<SettingsTab>(() => {
    lastTab = useApp.getState().settingsTab ?? lastTab;
    return lastTab;
  });
  const [settings, setSettings] = useState<SettingsView | null>(null);
  const settingsRef = useRef<SettingsView | null>(null);
  const escapeBlocked = useRef(false);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  const setTab = (next: SettingsTab) => {
    lastTab = next;
    setTabState(next);
  };

  useEffect(() => {
    let cancelled = false;
    api
      .settings_get()
      .then((v) => {
        if (cancelled) return;
        settingsRef.current = v;
        setSettings(v);
      })
      .catch((e) => !cancelled && toast(`${t("settings.loadFailed")} ${errorMessage(t, e)}`, "error"));
    return () => {
      cancelled = true;
    };
  }, []);

  // Escape closes the window, but a dialog or menu opened from inside it gets the key first. The
  // check runs while the key is travelling down (capture), before those layers close themselves.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape")
        escapeBlocked.current = document.querySelectorAll('[role="dialog"],[role="alertdialog"],[role="menu"]').length > 1;
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, []);
  const close = useCallback(() => {
    if (!escapeBlocked.current) closeRef.current();
  }, []);

  const update = useCallback(
    (patch: Partial<SettingsView>) => {
      const current = settingsRef.current;
      if (!current) return;
      const next = { ...current, ...patch };
      settingsRef.current = next;
      setSettings(next);
      api.settings_save(next).catch((e) => toast(errorMessage(t, e), "error"));
    },
    [t],
  );
  const updateTerminal = useCallback(
    (patch: Partial<TerminalSettings>) => {
      const current = settingsRef.current;
      if (current) update({ terminal: { ...current.terminal, ...patch } });
    },
    [update],
  );

  const onTabKey = (e: KeyboardEvent) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    e.preventDefault();
    const i = TABS.findIndex((x) => x.id === tab);
    const next = TABS[(i + (e.key === "ArrowRight" ? 1 : TABS.length - 1)) % TABS.length];
    setTab(next.id);
    requestAnimationFrame(() => document.getElementById(`settings-tab-${next.id}`)?.focus());
  };

  const pane = { settings, update, updateTerminal };
  const current = TABS.find((x) => x.id === tab)!;

  return (
    <Modal center onClose={close} closeOnBackdrop={false}>
      <div role="dialog" aria-modal aria-label={t("settings.title")} className={s.window}>
        <div className={s.toolbar}>
          <div className={s.titleRow}>
            <span className={s.title}>{t(current.label)}</span>
          </div>
          <div className={s.tabs} role="tablist" aria-label={t("settings.title")} onKeyDown={onTabKey}>
            {TABS.map((x) => (
              <button
                key={x.id}
                id={`settings-tab-${x.id}`}
                type="button"
                role="tab"
                aria-selected={tab === x.id}
                aria-controls="settings-panel"
                tabIndex={tab === x.id ? 0 : -1}
                className={cx(s.tab, tab === x.id && s.tabOn)}
                onClick={() => setTab(x.id)}
              >
                <Icon name={x.icon} />
                <span>{t(x.label)}</span>
              </button>
            ))}
          </div>
          <IconButton icon="x" label={t("btn.close")} className={s.close} onClick={() => closeRef.current()} />
        </div>
        <div id="settings-panel" role="tabpanel" aria-labelledby={`settings-tab-${tab}`} className={s.content}>
          {tab === "general" && <GeneralPane {...pane} />}
          {tab === "appearance" && <AppearancePane {...pane} />}
          {tab === "terminal" && <TerminalPane {...pane} />}
          {tab === "security" && <SecurityPane {...pane} />}
          {tab === "about" && <AboutPane />}
        </div>
      </div>
    </Modal>
  );
}
