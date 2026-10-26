/** The panel itself: the focus of its input, and whether it shows. */

import { useApp } from "@/app/store";
import { useTabs } from "@/app/tabs";
import { getSession } from "@/session/session";
import { HOME_SLOT, useAi } from "../store";

/** `selectAll`: select the draft, so typing replaces a suggested question. */
export function focusInput(selectAll = false) {
  useAi.setState((st) => ({ focusTick: st.focusTick + 1, focusSelectAll: selectAll }));
}

/** Shows or hides the panel (Ctrl+Shift+A / ⌘⇧A); the open state is a device preference (§9). */
export function toggleAiPanel(open?: boolean) {
  const app = useApp.getState();
  const next = open ?? !app.prefs.ai_panel_open;
  if (next === app.prefs.ai_panel_open) {
    if (next) focusInput();
    return;
  }
  void app.setPrefs({ ...app.prefs, ai_panel_open: next }).catch(() => {});
  if (next) focusInput();
  else {
    const active = useTabs.getState().active;
    if (active !== HOME_SLOT) getSession(active)?.focus();
  }
}
