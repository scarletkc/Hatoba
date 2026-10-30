import { useEffect } from "react";
import { Button, Icon } from "@hatoba/ui";
import { useT } from "@/i18n";
import { openExternal } from "@/lib/external";
import { REPO_URL, bugReportUrl } from "@/lib/github";
import { useStarPrompt, useStarPromptDue } from "./star";
import { useApp } from "./store";
import { useTabs } from "./tabs";
import { useOfferedUpdate } from "./update";
import s from "./StarPrompt.module.css";

/**
 * Asks for a GitHub star once it is due, on the home tab, and not while the update dot shows (spec §9). It stays in
 * the home page's lower right corner, over the page, until a button or closing it ends the prompt.
 */
export function StarPrompt() {
  const t = useT();
  const info = useApp((st) => st.info);
  const due = useStarPromptDue(useTabs((st) => st.connectedOnce));
  const home = useTabs((st) => st.active === "home");
  const updateAvailable = useOfferedUpdate() !== null;
  useEffect(() => {
    void useStarPrompt.getState().load();
  }, []);

  if (!home || updateAvailable || !due) return null;
  const { finish } = useStarPrompt.getState();
  const open = (url: string) => {
    finish();
    void openExternal(url);
  };
  return (
    <section className={s.star} aria-label={t("sidebar.star.title")}>
      <div className={s.head}>
        <Icon name="star" fill className={s.icon} />
        <span className={s.title}>{t("sidebar.star.title")}</span>
        <button type="button" className={s.close} title={t("btn.close")} aria-label={t("btn.close")} onClick={finish}>
          <Icon name="x" />
        </button>
      </div>
      <p className={s.body}>{t("sidebar.star.body")}</p>
      <div className={s.actions}>
        <Button size="sm" variant="primary" onClick={() => open(REPO_URL)}>
          {t("sidebar.star.star")}
        </Button>
        <Button size="sm" onClick={() => open(bugReportUrl(info))}>
          {t("sidebar.star.report")}
        </Button>
      </div>
    </section>
  );
}
