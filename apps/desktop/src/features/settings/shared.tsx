import type { ReactNode } from "react";
import { errorMessage } from "@/app/errors";
import { useApp } from "@/app/store";
import { Group, layoutStyles } from "@/components/layout";
import { toast } from "@/components/overlay";
import { HintText } from "@/components/patterns";
import { useT } from "@/i18n";
import type { LocalPrefs, SettingsView, TerminalSettings } from "@/ipc/types";
import s from "./shared.module.css";

/** What every settings pane gets: the synced settings (null while loading) and a way to change them. */
export interface PaneProps {
  settings: SettingsView | null;
  /** Saves the terminal fields in `patch`, and only those. */
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
        {hint && <HintText>{hint}</HintText>}
      </div>
      {children && <div className={s.control}>{children}</div>}
    </div>
  );
}

/** A titled group of rows; a pane's lone rows stay in an untitled `Group`. */
export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className={layoutStyles.section}>
      <div className={layoutStyles.sectionTitle}>{title}</div>
      <Group>{children}</Group>
    </section>
  );
}

export function Pane({ children }: { children: ReactNode }) {
  return <div className={s.pane}>{children}</div>;
}
