import { useEffect, useId, useRef } from "react";
import { Button, Icon, LinkButton, TextField } from "@/components/controls";
import { PopupSelect } from "@/components/overlay";
import { ErrorText, StatusLine } from "@/components/patterns";
import { errorMessage } from "@/app/errors";
import { Field } from "@/features/keys/Field";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AppError, SyncConfigInput } from "@/ipc/types";
import { isImeEvent } from "@/lib/ime";
import { API_TOKENS_URL, openExternal } from "./external";
import { EMPTY_D1, type D1Form } from "./wizardTypes";
import { WizardFrame, WizardTitle } from "./WizardFrame";
import s from "./Wizard.module.css";

/** Step 2b: D1 direct mode (spec §6.1, P1): Account ID + API Token, then pick a database. */
export function D1Step({
  form,
  update,
  onBack,
  onNext,
}: {
  form: D1Form;
  update: (patch: Partial<D1Form>) => void;
  onBack: () => void;
  onNext: (config: SyncConfigInput, join: boolean) => void;
}) {
  const t = useT();
  const accountId = useId();
  const tokenId = useId();
  const { verify } = form;
  const verified = verify.status === "ok";
  const canVerify = form.accountId.trim() && form.apiToken.trim() && verify.status !== "testing";
  const canContinue = verified && !!form.databaseId && !form.checking;
  const config = (databaseId: string): SyncConfigInput => ({
    kind: "d1",
    account_id: form.accountId.trim(),
    database_id: databaseId,
    api_token: form.apiToken.trim(),
  });

  const failure = (error: AppError | null) =>
    error?.code === "sync_offline"
      ? { message: errorMessage(t, error), offline: true }
      : { message: t("sync.token.err"), offline: false };

  // Only the latest request's answer counts, for the credentials in the fields: whether the
  // database holds a vault decides the flow. A new request, an edit and leaving the step all end
  // the one in flight; the wizard keeps `form`, so a late answer would land in the next visit.
  // What was still running when the step comes back is one of those: a token check can be run
  // again, and the chosen database is checked again.
  const epoch = useRef(0);
  useEffect(() => {
    const current = epoch;
    return () => {
      current.current++;
    };
  }, []);

  const pickDatabase = async (databaseId: string) => {
    const n = ++epoch.current;
    update({ databaseId, initialized: false, checking: true });
    try {
      const result = await api.sync_test(config(databaseId));
      if (n === epoch.current) update({ initialized: result.ok && result.initialized, checking: false });
    } catch {
      if (n === epoch.current) update({ checking: false });
    }
  };

  useEffect(() => {
    if (verify.status === "testing") update({ verify: { status: "idle" } });
    if (form.checking) void pickDatabase(form.databaseId);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const runVerify = async () => {
    if (!canVerify) return;
    const n = ++epoch.current;
    update({ verify: { status: "testing" }, databaseId: "", databases: [], initialized: false });
    try {
      const result = await api.sync_test(config(""));
      if (n !== epoch.current) return;
      if (!result.ok) return update({ verify: { status: "failed", ...failure(result.error) } });
      const databases = result.databases ?? [];
      update({ verify: { status: "ok", result }, databases });
      if (databases.length === 1) void pickDatabase(databases[0].id);
    } catch (e) {
      if (n === epoch.current) update({ verify: { status: "failed", ...failure(toAppError(e)) } });
    }
  };

  const reset = (patch: Partial<D1Form>) => {
    epoch.current++;
    update({ ...EMPTY_D1, accountId: form.accountId, apiToken: form.apiToken, ...patch });
  };

  const verifiedLine =
    verify.status === "ok"
      ? verify.result.latency_ms !== null
        ? t("sync.d1.verified", { n: form.databases.length, ms: verify.result.latency_ms })
        : t("sync.d1.verifiedNoMs")
      : null;

  return (
    <WizardFrame
      step={2}
      footer={
        <>
          <Button onClick={onBack}>{t("btn.back")}</Button>
          <Button variant="primary" disabled={!canContinue} onClick={() => onNext(config(form.databaseId), form.initialized)}>
            {t("btn.continue")}
          </Button>
        </>
      }
    >
      <WizardTitle title={t("sync.d1.title")} body={t("sync.d1.body")} />
      <div className={s.fields}>
        <Field label={t("sync.accountId")} htmlFor={accountId}>
          <TextField
            id={accountId}
            mono
            large
            value={form.accountId}
            onChange={(e) => reset({ accountId: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void runVerify()}
          />
        </Field>

        <Field label={t("sync.apiToken")} htmlFor={tokenId}>
          <TextField
            id={tokenId}
            large
            secret
            value={form.apiToken}
            invalid={verify.status === "failed"}
            onChange={(e) => reset({ apiToken: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void runVerify()}
          />
          {verify.status === "failed" && (
            <ErrorText icon="warning-circle" className={s.tokenError}>
              <div>{verify.message}</div>
              <LinkButton onClick={() => void openExternal(API_TOKENS_URL)}>
                <span className={s.externalLink}>
                  {t("sync.token.edit")}
                  <Icon name="arrow-up-right" />
                </span>
              </LinkButton>
            </ErrorText>
          )}
          <div className={s.testRow}>
            <Button size="sm" icon="seal-check" busy={verify.status === "testing"} disabled={!canVerify} onClick={() => void runVerify()}>
              {t("sync.d1.verify")}
            </Button>
            {verifiedLine && (
              <div className={s.testStatus} role="status">
                <StatusLine tone="success">{verifiedLine}</StatusLine>
              </div>
            )}
          </div>
        </Field>

        <Field
          label={t("sync.database")}
          hint={form.initialized ? t("sync.d1.hasVault") : verified && form.databases.length === 0 ? t("sync.d1.noDb") : undefined}
        >
          <PopupSelect
            ariaLabel={t("sync.database")}
            icon="database"
            value={form.databaseId}
            disabled={!verified || form.databases.length === 0}
            placeholder={verified ? t("sync.database.choose") : t("sync.database.pending")}
            options={form.databases.map((d) => ({ value: d.id, label: d.name, hint: d.region ?? undefined }))}
            onChange={(id) => void pickDatabase(id)}
          />
        </Field>
      </div>
    </WizardFrame>
  );
}
