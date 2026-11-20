import { parseMessage } from "@/lib/ai/attachments";
import { shownCall } from "@/lib/ai/mcpCall";
import type { HatobaApi } from "../api";
import type { AiAutoVerdict } from "../types";
import { lastUser, type AiMockCtx, type Conv } from "./aiShared";

export function createAiJudge(ctx: AiMockCtx): Pick<HatobaApi, "ai_judge"> {
  const { deps, flags, verdicts, delay, need, answered, openCall } = ctx;

  /** Like Rust's `question_ids`: the questions a call of `tool` gets, in the judge's order. */
  function judgeQuestions(tool: string): string[] {
    if (tool === "edit_file" || tool === "write_file")
      return ["requested", "forbidden", "touches_secrets", "security_change", "sensitive_file"];
    if (tool === "fetch_url") return ["forbidden", "url_carries_data"];
    return [
      "requested",
      "forbidden",
      "changes_anything",
      "destroys_data",
      "touches_secrets",
      "sends_data_out",
      "security_change",
      "runs_unseen_code",
    ];
  }

  /**
   * The mock judge's answers, from a few words, so that the browser preview shows both verdicts: a command of
   * reading words runs, while `rm`, `restart`, `install`, `curl`, a redirection and the like ask, unless the
   * user's last message names what the call works on, which counts as asking for it.
   */
  function mockAnswers(c: Conv, tool: string, args: Record<string, unknown>): Record<string, number> {
    const text = [args.command, args.text, args.path, args.url, args.content]
      .filter((v): v is string => typeof v === "string")
      .join("\n");
    // What an edit writes, which only `security_change` asks about.
    const written = typeof args.new_string === "string" ? `${text}\n${args.new_string}` : text;
    const asked = parseMessage(lastUser(c).text).typed.toLowerCase();
    const named = text
      .toLowerCase()
      .split(/[^\p{L}\p{N}_.-]+/u)
      .some((w) => w.length > 2 && !["sudo", "systemctl"].includes(w) && asked.includes(w));
    const yes = (re: RegExp, p: number) => (re.test(text) ? p : 0.03);
    return {
      requested: named || tool.startsWith("mcp__") ? 0.92 : 0.35,
      forbidden: 0.02,
      changes_anything: yes(
        /\b(restart|reload|stop|start|enable|disable|install|remove|purge|apt|apt-get|yum|dnf|chmod|chown|useradd|usermod)\b|sed\s+-i|>/,
        0.78,
      ),
      destroys_data: yes(/\b(rm|truncate|prune|vacuum|drop|delete)\b|(^|[^>])>(?!>)/, 0.66),
      touches_secrets: yes(/\.ssh|id_rsa|id_ed25519|shadow|passw|token|secret|\.env\b/i, 0.72),
      sends_data_out: yes(/\b(curl|wget|scp|rsync|nc|ssh)\b/, 0.58),
      security_change:
        /\b(chmod|chown|useradd|usermod|passwd|visudo|crontab|ufw|iptables|setenforce)\b|authorized_keys|sudoers|systemctl\s+enable|strict-ssl\s*=\s*false|sslVerify\s*=\s*false|check_certificate\s*=\s*off|StrictHostKeyChecking\s+no|PermitRootLogin\s+yes/.test(
          written,
        )
          ? 0.74
          : 0.03,
      runs_unseen_code: yes(/\|\s*(sudo\s+)?(ba|z)?sh\b|base64\s+(-d|--decode)|\b(ba|z)?sh\s+\S+\.sh\b/, 0.83),
      sensitive_file: yes(/authorized_keys|sudoers|cron|systemd|\.bashrc|\.profile|\.zshrc|rc\.local|\/etc\/pam/, 0.81),
      url_carries_data: yes(/[?&][^=&]*(token|key|secret|password)=/i, 0.7),
    };
  }

  /** Like Rust's `judge::decide` with a System One endpoint's thresholds: the decision and what kept the call from running. */
  function mockDecide(tool: string, answers: Record<string, number>): Pick<AiAutoVerdict, "decision" | "concerns"> {
    const kind = tool === "edit_file" || tool === "write_file" ? "file" : tool === "fetch_url" ? "fetch" : "command";
    const hazard = (id: string) => (answers[id] ?? 1) >= 0.1;
    const unrequested = (answers.requested ?? 0) < 0.85;
    const unrequestedChange = kind === "command" && hazard("changes_anything") && unrequested;
    const concerns = judgeQuestions(tool).filter((id) =>
      id === "requested"
        ? kind === "command"
          ? unrequestedChange
          : unrequested
        : id === "changes_anything"
          ? unrequestedChange
          : hazard(id),
    );
    return { decision: concerns.length === 0 ? "run" : "ask", concerns };
  }

  return {
    // AI-47: the mock judge (`mockAnswers`). `?ai=judgeslow` takes 4 seconds, and `?ai=judgefail` times out.
    ai_judge: async (id, callId) => {
      const c = need(id);
      const x = openCall(c, callId);
      const judge = (await deps.settings()).judge_model;
      const failed = (failure: string): AiAutoVerdict => ({
        provider_id: judge?.provider_id ?? "",
        model_id: judge?.model_id ?? "",
        decision: "ask",
        answers: [],
        concerns: [],
        failure,
      });
      let verdict: AiAutoVerdict;
      if (!judge) verdict = failed("unavailable");
      else {
        await delay(flags.has("judgeslow") ? 4000 : 700);
        // A stop stored a cancelled result while the judge answered.
        if (answered(c).has(callId)) verdict = failed("cancelled");
        else if (flags.has("judgefail")) verdict = failed("timeout");
        else {
          // AI-45: a `call_mcp_tool` call is judged as the tool it names.
          const shown = shownCall(x);
          let args: Record<string, unknown> = {};
          try {
            args = JSON.parse(shown.arguments) as Record<string, unknown>;
          } catch {
            // cut-off arguments name nothing
          }
          const answers = mockAnswers(c, shown.name, args);
          verdict = {
            provider_id: judge.provider_id,
            model_id: judge.model_id,
            answers: judgeQuestions(shown.name).map((question) => ({ question, probability: answers[question] })),
            failure: null,
            ...mockDecide(shown.name, answers),
          };
        }
      }
      verdicts.set(callId, verdict);
      return structuredClone(verdict);
    },
  };
}
