import type { MessageKey, Params } from "@/i18n";
import type { AiAutoVerdict, AiJudgeAnswer } from "@/ipc/types";
import type { HardRule } from "./autoRules";
import type { AutoAsk } from "./turn";

type Tr = (key: MessageKey, params?: Params) => string;

/** AI-47: the failure of a verdict the panel makes itself when asking the judge failed; Rust never stores it. */
export const JUDGE_ERROR = "error";

/** The judge's questions as an answer list names them (AI-47). */
const QUESTION: Record<string, MessageKey> = {
  requested: "ai.auto.q.requested",
  forbidden: "ai.auto.q.forbidden",
  changes_anything: "ai.auto.q.changes_anything",
  destroys_data: "ai.auto.q.destroys_data",
  touches_secrets: "ai.auto.q.touches_secrets",
  sends_data_out: "ai.auto.q.sends_data_out",
  security_change: "ai.auto.q.security_change",
  runs_unseen_code: "ai.auto.q.runs_unseen_code",
  sensitive_file: "ai.auto.q.sensitive_file",
  url_carries_data: "ai.auto.q.url_carries_data",
};

/**
 * What concerned the judge, per question (AI-47): for `requested` that the user did not clearly ask for the call,
 * for the others that the call may do what the question asks about.
 */
const CONCERN: Record<string, MessageKey> = {
  requested: "ai.auto.concern.requested",
  forbidden: "ai.auto.concern.forbidden",
  changes_anything: "ai.auto.concern.changes_anything",
  destroys_data: "ai.auto.concern.destroys_data",
  touches_secrets: "ai.auto.concern.touches_secrets",
  sends_data_out: "ai.auto.concern.sends_data_out",
  security_change: "ai.auto.concern.security_change",
  runs_unseen_code: "ai.auto.concern.runs_unseen_code",
  sensitive_file: "ai.auto.concern.sensitive_file",
  url_carries_data: "ai.auto.concern.url_carries_data",
};

/** Why the judge could not judge (AI-47); a kind a newer version added reads as `ai.auto.failure.other`. */
const FAILURE: Record<string, MessageKey> = {
  unavailable: "ai.auto.failure.unavailable",
  too_long: "ai.auto.failure.too_long",
  timeout: "ai.auto.failure.timeout",
  unreachable: "ai.auto.failure.unreachable",
  refused: "ai.auto.failure.refused",
  malformed: "ai.auto.failure.malformed",
  cancelled: "ai.auto.failure.cancelled",
  [JUDGE_ERROR]: "ai.auto.failure.error",
};

/** A probability as a whole percentage. */
export function percent(p: number): string {
  return `${Math.round(Math.min(1, Math.max(0, p)) * 100)}%`;
}

/** The judge's answers in its order, such as "requested 96% · changes something 71%"; an unknown question by its id. */
export function answersText(t: Tr, answers: readonly AiJudgeAnswer[]): string {
  return answers.map((a) => `${QUESTION[a.question] ? t(QUESTION[a.question]) : a.question} ${percent(a.probability)}`).join(" · ");
}

/**
 * What concerned the judge, in its order, each with the probability of yes it gave (AI-47): "may delete or
 * overwrite data (64%), not clearly asked for (40%)". A concern without an answer (one that was missing) has no
 * percentage.
 */
export function concernsText(t: Tr, verdict: AiAutoVerdict): string {
  return verdict.concerns
    .map((id) => {
      const what = CONCERN[id] ? t(CONCERN[id]) : id;
      const answer = verdict.answers.find((a) => a.question === id);
      return answer ? t("ai.auto.percent", { what, p: percent(answer.probability) }) : what;
    })
    .join(t("ai.auto.separator"));
}

/** Why the judge's verdict asks: what concerned it, or why it could not judge. */
export function verdictText(t: Tr, verdict: AiAutoVerdict): string {
  if (verdict.failure !== null) return t(FAILURE[verdict.failure] ?? "ai.auto.failure.other");
  if (verdict.concerns.length === 0) return t("ai.auto.failure.other");
  return t("ai.auto.askedBecause", { concerns: concernsText(t, verdict) });
}

/** Why a hard rule sends a call straight to the approval card (AI-47). */
export function ruleText(t: Tr, rule: HardRule): string {
  switch (rule.kind) {
    case "mcp_always_ask":
      return t("ai.auto.rule.mcpAlwaysAsk");
    case "mcp_unknown":
      return t("ai.auto.rule.mcpUnknown");
    case "hidden":
      return t("ai.auto.rule.hidden");
    case "command":
      return t("ai.auto.rule.command", { command: rule.command });
    case "password":
      return t("ai.auto.rule.password");
    case "after_reject":
      return t("ai.auto.rule.afterReject");
  }
}

/** What the approval card of a call in Auto mode says about why it asks (AI-47). */
export function askText(t: Tr, ask: AutoAsk): string {
  return "rule" in ask ? ruleText(t, ask.rule) : verdictText(t, ask.verdict);
}
