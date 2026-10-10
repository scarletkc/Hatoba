import { useEffect, useState } from "react";
import { Icon, TextField } from "@/components/controls";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { COMMON_PASSWORDS } from "./commonPasswords";
import s from "./PasswordStrength.module.css";

export const MIN_PASSWORD_LENGTH = 8;
/** zxcvbn score (0–4) a new master password must reach (SEC-09). */
export const MIN_SCORE = 2;

type Checker = (password: string) => number;
let checkerPromise: Promise<Checker> | null = null;

/** Lazy-loads zxcvbn and its dictionaries on first use so they stay out of the startup bundle. */
function loadChecker(): Promise<Checker> {
  checkerPromise ??= Promise.all([
    import("@zxcvbn-ts/core"),
    import("@zxcvbn-ts/language-common"),
    import("@zxcvbn-ts/language-en"),
  ]).then(([{ ZxcvbnFactory }, common, en]) => {
    const zxcvbn = new ZxcvbnFactory({
      dictionary: { ...common.dictionary, ...en.dictionary, hatoba: COMMON_PASSWORDS },
      graphs: common.adjacencyGraphs,
    });
    // Long inputs are truncated: zxcvbn's cost grows with length and 100 characters is already off the scale.
    return (password: string) => zxcvbn.check(password.slice(0, 100)).score;
  });
  return checkerPromise;
}

/** zxcvbn score for `password`, or `null` while the library is loading / the field is empty. */
export function useStrength(password: string): number | null {
  const [result, setResult] = useState<{ password: string; score: number } | null>(null);
  useEffect(() => {
    if (!password) return;
    let cancelled = false;
    void loadChecker().then((check) => {
      if (!cancelled) setResult({ password, score: check(password) });
    });
    return () => {
      cancelled = true;
    };
  }, [password]);
  return password && result?.password === password ? result.score : null;
}

const LABELS = ["vault.strength.weak", "vault.strength.weak", "vault.strength.fair", "vault.strength.strong", "vault.strength.veryStrong"] as const;
const FILLED = [1, 1, 2, 3, 4];

/** The design's 4-segment meter with a label (design §05 step 3). */
export function StrengthMeter({ score }: { score: number | null }) {
  const t = useT();
  const color = score === null ? "var(--fill2)" : score <= 1 ? "var(--red)" : score === 2 ? "var(--orange)" : "var(--green)";
  const filled = score === null ? 0 : FILLED[score];
  return (
    <div className={s.meter} aria-live="polite">
      <div className={s.segments}>
        {[0, 1, 2, 3].map((i) => (
          <span key={i} className={s.segment} style={{ background: i < filled ? color : undefined }} />
        ))}
      </div>
      <span className={s.label}>{score === null ? "" : t(LABELS[score])}</span>
    </div>
  );
}

export type NewPasswordIssue = "short" | "weak" | null;

/** State for a "new master password + confirmation" form. */
export function useNewPassword() {
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  // Start loading zxcvbn as soon as the form appears so the first keystrokes get a score.
  useEffect(() => void loadChecker(), []);
  const score = useStrength(password);
  const issue: NewPasswordIssue =
    password.length < MIN_PASSWORD_LENGTH ? "short" : score !== null && score < MIN_SCORE ? "weak" : null;
  const matches = confirm.length > 0 && confirm === password;
  const valid = issue === null && score !== null && matches;
  return { password, setPassword, confirm, setConfirm, score, issue, matches, valid };
}

export type NewPasswordState = ReturnType<typeof useNewPassword>;

/** Master password + strength meter + confirmation, as in the design's wizard step 3. */
export function NewPasswordFields({
  state,
  passwordLabel,
  confirmLabel,
  autoFocus,
  disabled,
  onEnter,
}: {
  state: NewPasswordState;
  passwordLabel: string;
  confirmLabel: string;
  autoFocus?: boolean;
  disabled?: boolean;
  onEnter?: () => void;
}) {
  const t = useT();
  const { password, confirm, score, issue, matches } = state;
  const [confirmTouched, setConfirmTouched] = useState(false);
  const mismatch = confirm.length > 0 && !matches && (confirmTouched || confirm.length >= password.length);
  const enter = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !isImeEvent(e)) onEnter?.();
  };
  return (
    <>
      <div className={s.field}>
        <label className={s.label2} htmlFor="new-password">
          {passwordLabel}
        </label>
        <TextField
          id="new-password"
          large
          secret
          autoFocus={autoFocus}
          autoComplete="new-password"
          readOnly={disabled}
          value={password}
          onChange={(e) => state.setPassword(e.target.value)}
          onKeyDown={enter}
        />
        <StrengthMeter score={password ? score : null} />
        {password.length > 0 && issue && (
          <div className={cx(s.note, issue === "weak" && s.noteWarn)}>
            {issue === "short" ? t("vault.err.tooShort") : t("vault.err.tooWeak")}
          </div>
        )}
      </div>
      <div className={s.field}>
        <label className={s.label2} htmlFor="confirm-password">
          {confirmLabel}
        </label>
        <TextField
          id="confirm-password"
          large
          secret
          autoComplete="new-password"
          readOnly={disabled}
          invalid={mismatch}
          value={confirm}
          trailing={matches ? <Icon name="check-circle" fill size={15} color="var(--green)" /> : undefined}
          onChange={(e) => state.setConfirm(e.target.value)}
          onBlur={() => setConfirmTouched(true)}
          onKeyDown={enter}
        />
        {mismatch && (
          <div className={s.error} role="alert">
            {t("vault.err.mismatch")}
          </div>
        )}
      </div>
    </>
  );
}
