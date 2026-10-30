import { HintText, LinkButton } from "@hatoba/ui";
import { useApp } from "@/app/store";
import { useT } from "@/i18n";
import type { AiAutoVerdict } from "@/ipc/types";
import { answersText, askText, verdictText } from "../rules/autoText";
import { MODE_ICON } from "../tools";
import type { AutoAsk } from "../turn";
import s from "../messages/Messages.module.css";

/**
 * AI-47: in the block of a call Auto decided, what the judge decided: that Auto ran it without asking, or what
 * concerned the judge (or why it could not judge) when the card asked, with the judge's answers.
 */
export function AutoVerdict({ verdict }: { verdict: AiAutoVerdict }) {
  const t = useT();
  const answers = answersText(t, verdict.answers);
  return (
    <div className={s.autoVerdict}>
      <div className={s.label}>{t("ai.mode.auto")}</div>
      <HintText>{verdict.decision === "run" ? t("ai.auto.ran") : verdictText(t, verdict)}</HintText>
      {answers && <HintText>{t("ai.auto.answers", { answers })}</HintText>}
    </div>
  );
}

/**
 * AI-47: on the approval card of a call in Auto mode, why it asks: a hard rule, what concerned the judge (its
 * answers on hover), or why the judge could not judge, with a way to Settings → AI when no judge is available.
 */
export function AutoReason({ ask }: { ask: AutoAsk }) {
  const t = useT();
  const verdict = "verdict" in ask ? ask.verdict : null;
  return (
    <div title={verdict ? answersText(t, verdict.answers) || undefined : undefined}>
      <HintText icon={MODE_ICON.auto} className={s.autoReason}>
        {askText(t, ask)}
        {verdict?.failure === "unavailable" && (
          <>
            {" "}
            <LinkButton className={s.inlineLink} onClick={() => useApp.getState().openSettings(true, "ai")}>
              {t("ai.noProvider.open")}
            </LinkButton>
          </>
        )}
      </HintText>
    </div>
  );
}
