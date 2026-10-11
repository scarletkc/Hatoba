import type { ReactNode } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { toast } from "@/components/overlay";
import { useT } from "@/i18n";
import type { LocalPrefs, SettingsView, TerminalSettings } from "@/ipc/types";
import s from "./shared.module.css";

/** What every settings pane gets: the synced settings (null while loading) and ways to change them. */
export interface PaneProps {
  settings: SettingsView | null;
  update(patch: Partial<SettingsView>): void;
  updateTerminal(patch: Partial<TerminalSettings>): void;
}

/** Device-local prefs (theme, density, language, …) applied instantly and persisted. */
export function usePrefs() {
  const t = useT();
  const prefs = useApp((st) => st.prefs);
  const set = (patch: Partial<LocalPrefs>) => {
    useApp
      .getState()
      .setPrefs({ ...useApp.getState().prefs, ...patch })
      .catch((e) => toast(errorMessage(t, e), "error"));
  };
  return [prefs, set] as const;
}

/** One settings row: label (+ optional hint) on the left, control on the right. */
export function SettingRow({
  label,
  hint,
  children,
  top,
}: {
  label: ReactNode;
  hint?: ReactNode;
  children?: ReactNode;
  top?: boolean;
}) {
  return (
    <div className={top ? `${s.row} ${s.rowTop}` : s.row}>
      <div className={s.text}>
        <span className={s.label}>{label}</span>
        {hint && <span className={s.hint}>{hint}</span>}
      </div>
      {children && <div className={s.control}>{children}</div>}
    </div>
  );
}

export function Pane({ children }: { children: ReactNode }) {
  return <div className={s.pane}>{children}</div>;
}
