import { useEffect, useId, useRef, useState, type Dispatch, type SetStateAction } from "react";
import { Button, Icon, LinkButton, Spinner, TextField } from "@/components/controls";
import { Callout } from "@/components/layout";
import { PopupSelect } from "@/components/overlay";
import { errorMessage } from "@/app/errors";
import { Field } from "@/features/keys/Field";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AppError, DeployPlan, DeployStep as Step, DeployTarget } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { API_TOKENS_URL, DEPLOY_TOKEN_URL, openExternal } from "./external";
import type { T } from "./syncUtils";
import { DEPLOY_STEPS, EMPTY_DEPLOY, type DeployForm, type StepState } from "./wizardTypes";
import { WizardFrame, WizardTitle } from "./WizardFrame";
import s from "./Wizard.module.css";

type Set = Dispatch<SetStateAction<DeployForm>>;

/** A lowercase DNS label, as Cloudflare requires for Worker names and workers.dev subdomains. */
export const LABEL = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/;
const DATABASE = /^[a-z0-9_-]{1,60}$/;

/** Deployment failures in the words of this flow; anything else uses the shared error text. */
export function deployError(t: T, e: AppError): string {
  if (e.code === "sync_offline") return t("sync.dep.offline");
  if (e.code === "invalid_input" && e.field) return t(`sync.dep.invalid.${e.field}` as MessageKey);
  return errorMessage(t, e);
}

/** Errors the user fixes by editing the token in the Cloudflare dashboard. */
export function EditTokenLink({ error }: { error: AppError | null }) {
  const t = useT();
  if (error?.code !== "cloudflare_token" && error?.code !== "cloudflare_permission") return null;
  return (
    <LinkButton className={s.guideLink} onClick={() => void openExternal(API_TOKENS_URL)}>
      <span className={s.externalLink}>
        {t("sync.token.edit")}
        <Icon name="arrow-up-right" />
      </span>
    </LinkButton>
  );
}

const target = (f: DeployForm): DeployTarget => ({
  account_id: f.accountId.trim(),
  worker_name: f.workerName.trim(),
  database_name: f.databaseName.trim(),
  subdomain: f.plan && !f.plan.subdomain ? f.subdomain.trim() || null : null,
});

/**
 * Step 2, in-app deployment (spec §6.7): the API token, a review of what step 2 found, then the
 * deployment steps with their progress. The wizard keeps `form`, so leaving for the password step
 * and coming back keeps the deployment.
 */
export function DeployStep({ form, set, onBack, onReady }: { form: DeployForm; set: Set; onBack: () => void; onReady: (handle: string) => void }) {
  if (form.phase === "run" && form.start) return <RunPhase form={form} set={set} handle={form.start.handle} onReady={onReady} />;
  if (form.phase === "review" && form.start) return <ReviewPhase form={form} set={set} />;
  return <TokenPhase form={form} set={set} onBack={onBack} />;
}

function TokenPhase({ form, set, onBack }: { form: DeployForm; set: Set; onBack: () => void }) {
  const t = useT();
  const tokenId = useId();
  const accountId = useId();
  const tokenRef = useRef<HTMLInputElement>(null);
  const canVerify = form.apiToken.trim().length > 0 && !form.verifying;

  useEffect(() => {
    tokenRef.current?.focus();
  }, []);

  const verify = async () => {
    if (!canVerify) return;
    set((f) => ({ ...f, verifying: true, tokenError: null }));
    try {
      const start = await api.deploy_start(form.apiToken.trim(), form.accountId.trim() || null);
      // DEPLOY-07: Rust has the token now; the WebView forgets it.
      set((f) => ({
        ...f,
        apiToken: "",
        verifying: false,
        start,
        phase: "review",
        accountId: f.accountId.trim() || (start.accounts.length === 1 ? start.accounts[0].id : ""),
        workerName: f.workerName || start.worker_name,
        databaseName: f.databaseName || start.database_name,
        plan: null,
        planError: null,
      }));
    } catch (e) {
      set((f) => ({ ...f, verifying: false, tokenError: toAppError(e) }));
      tokenRef.current?.focus();
    }
  };

  return (
    <WizardFrame
      step={2}
      footer={
        <>
          <Button onClick={onBack} disabled={form.verifying}>
            {t("btn.back")}
          </Button>
          <Button variant="primary" busy={form.verifying} disabled={!canVerify} onClick={() => void verify()}>
            {t("btn.continue")}
          </Button>
        </>
      }
    >
      <WizardTitle title={t("sync.dep.title")} body={t("sync.dep.body")} />
      <Callout icon="key" title={t("sync.dep.token.title")}>
        <span className={s.lossBody}>{t("sync.dep.token.help")}</span>
        <div className={s.calloutLink}>
          <LinkButton onClick={() => void openExternal(DEPLOY_TOKEN_URL)}>
            <span className={s.externalLink}>
              {t("sync.dep.createToken")}
              <Icon name="arrow-up-right" />
            </span>
          </LinkButton>
        </div>
      </Callout>
      <div className={s.fields}>
        <Field
          label={t("sync.apiToken")}
          htmlFor={tokenId}
          error={form.tokenError ? deployError(t, form.tokenError) : undefined}
          hint={t("sync.dep.token.memory")}
        >
          <TextField
            id={tokenId}
            ref={tokenRef}
            large
            secret
            autoComplete="off"
            value={form.apiToken}
            disabled={form.verifying}
            invalid={!!form.tokenError}
            onChange={(e) => set((f) => ({ ...f, apiToken: e.target.value, tokenError: null }))}
            onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void verify()}
          />
        </Field>
        <EditTokenLink error={form.tokenError} />
        <Field label={t("sync.dep.accountId")} htmlFor={accountId} hint={t("sync.dep.accountId.hint")}>
          <TextField
            id={accountId}
            mono
            large
            value={form.accountId}
            disabled={form.verifying}
            onChange={(e) => set((f) => ({ ...f, accountId: e.target.value, tokenError: null }))}
            onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void verify()}
          />
        </Field>
      </div>
    </WizardFrame>
  );
}

function ReviewPhase({ form, set }: { form: DeployForm; set: Set }) {
  const t = useT();
  const workerId = useId();
  const databaseId = useId();
  const subdomainId = useId();
  const accountId = useId();
  const start = form.start!;
  const request = useRef(0);
  const inspected = useRef("");
  const { plan } = form;

  const nameError = (() => {
    if (form.workerName && !LABEL.test(form.workerName.trim())) return { field: "worker", text: t("sync.dep.invalid.worker_name") };
    if (form.databaseName && !DATABASE.test(form.databaseName.trim())) return { field: "database", text: t("sync.dep.invalid.database_name") };
    return null;
  })();
  const subdomainNeeded = !!plan && !plan.subdomain;
  const subdomainOk = LABEL.test(form.subdomain.trim());
  const blocked = plan?.worker === "has_vault" || plan?.worker === "foreign";
  const canDeploy = !!plan && !blocked && !form.inspecting && !nameError && (!subdomainNeeded || subdomainOk);

  const inspect = async (f: DeployForm) => {
    const t2 = target(f);
    if (!t2.account_id || !LABEL.test(t2.worker_name) || !DATABASE.test(t2.database_name)) return;
    const key = `${t2.account_id}|${t2.worker_name}|${t2.database_name}`;
    inspected.current = key;
    const n = ++request.current;
    set((x) => ({ ...x, inspecting: true, planError: null }));
    try {
      const next = await api.deploy_inspect(start.handle, t2);
      if (n !== request.current) return;
      set((x) => ({ ...x, inspecting: false, plan: next, namesOpen: x.namesOpen || next.worker === "has_vault" || next.worker === "foreign" }));
    } catch (e) {
      if (n !== request.current) return;
      set((x) => ({ ...x, inspecting: false, plan: null, planError: toAppError(e) }));
    }
  };

  // Step 2 runs as soon as the account is known.
  useEffect(() => {
    if (!form.plan && !form.planError) void inspect(form);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const reinspect = (patch: Partial<DeployForm>) => {
    const next = { ...form, ...patch };
    const key = `${next.accountId.trim()}|${next.workerName.trim()}|${next.databaseName.trim()}`;
    if (key !== inspected.current) void inspect(next);
  };

  const back = () => {
    void api.deploy_cancel(start.handle);
    set((f) => ({ ...EMPTY_DEPLOY, accountId: start.account_owned ? f.accountId : "", workerName: f.workerName, databaseName: f.databaseName }));
  };

  const deploy = () => {
    if (!canDeploy) return;
    set((f) => ({ ...f, phase: "run", run: "idle", steps: {}, runError: null }));
  };

  const url = plan?.subdomain
    ? `https://${form.workerName.trim()}.${plan.subdomain}.workers.dev`
    : subdomainOk
      ? `https://${form.workerName.trim()}.${form.subdomain.trim()}.workers.dev`
      : null;

  return (
    <WizardFrame
      step={2}
      footer={
        <>
          <Button onClick={back}>{t("btn.back")}</Button>
          <Button variant="primary" disabled={!canDeploy} onClick={deploy}>
            {t("sync.dep.deploy")}
          </Button>
        </>
      }
    >
      <WizardTitle title={t("sync.dep.review.title")} body={t("sync.dep.review.body")} />
      <div className={s.fields}>
        <Field label={t("sync.dep.account")} htmlFor={accountId}>
          {start.accounts.length > 1 ? (
            <PopupSelect
              ariaLabel={t("sync.dep.account")}
              icon="buildings"
              value={form.accountId}
              placeholder={t("sync.dep.account.choose")}
              options={start.accounts.map((a) => ({ value: a.id, label: a.name || a.id, hint: a.name ? a.id.slice(0, 8) : undefined }))}
              onChange={(id) => {
                set((f) => ({ ...f, accountId: id, plan: null }));
                reinspect({ accountId: id });
              }}
            />
          ) : start.accounts.length === 1 || start.account_owned ? (
            <div className={s.accountLine}>
              {start.accounts[0]?.name && <span>{start.accounts[0].name}</span>}
              <span className={cx(s.mono, s.dim)}>{form.accountId}</span>
            </div>
          ) : (
            <TextField
              id={accountId}
              mono
              large
              value={form.accountId}
              placeholder={t("sync.dep.accountId.enter")}
              onChange={(e) => set((f) => ({ ...f, accountId: e.target.value }))}
              onBlur={() => reinspect({})}
              onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && reinspect({})}
            />
          )}
        </Field>

        <PlanSummary t={t} form={form} plan={plan} url={url} onRetry={() => void inspect(form)} />

        {subdomainNeeded && (
          <Field
            label={t("sync.dep.subdomain")}
            htmlFor={subdomainId}
            hint={t("sync.dep.subdomain.hint")}
            error={form.subdomain && !subdomainOk ? t("sync.dep.invalid.subdomain") : undefined}
          >
            <TextField
              id={subdomainId}
              mono
              large
              value={form.subdomain}
              invalid={!!form.subdomain && !subdomainOk}
              trailing={<span className={s.fieldSuffix}>.workers.dev</span>}
              onChange={(e) => set((f) => ({ ...f, subdomain: e.target.value.toLowerCase() }))}
              onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && deploy()}
            />
          </Field>
        )}

        <div className={s.wrangler}>
          <button type="button" className={s.disclosure} aria-expanded={form.namesOpen} onClick={() => set((f) => ({ ...f, namesOpen: !f.namesOpen }))}>
            <Icon name={form.namesOpen ? "caret-down" : "caret-right"} />
            {t("sync.dep.names")}
          </button>
          {form.namesOpen && (
            <div className={s.fields}>
              <Field label={t("sync.dep.workerName")} htmlFor={workerId} error={nameError?.field === "worker" ? nameError.text : undefined}>
                <TextField
                  id={workerId}
                  mono
                  value={form.workerName}
                  invalid={nameError?.field === "worker"}
                  onChange={(e) => set((f) => ({ ...f, workerName: e.target.value.toLowerCase() }))}
                  onBlur={() => reinspect({})}
                  onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && reinspect({})}
                />
              </Field>
              <Field label={t("sync.dep.databaseName")} htmlFor={databaseId} error={nameError?.field === "database" ? nameError.text : undefined}>
                <TextField
                  id={databaseId}
                  mono
                  value={form.databaseName}
                  invalid={nameError?.field === "database"}
                  onChange={(e) => set((f) => ({ ...f, databaseName: e.target.value.toLowerCase() }))}
                  onBlur={() => reinspect({})}
                  onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && reinspect({})}
                />
              </Field>
            </div>
          )}
        </div>
      </div>
    </WizardFrame>
  );
}

function PlanSummary({ t, form, plan, url, onRetry }: { t: T; form: DeployForm; plan: DeployPlan | null; url: string | null; onRetry: () => void }) {
  if (form.inspecting)
    return (
      <div className={s.planStatus} role="status">
        <Spinner />
        {t("sync.dep.checking")}
      </div>
    );
  if (form.planError)
    return (
      <div className={s.tokenError} role="alert">
        <Icon name="warning-circle" />
        <div className={s.tokenErrorText}>
          <div>{deployError(t, form.planError)}</div>
          <div className={s.linkRow}>
            <EditTokenLink error={form.planError} />
            <LinkButton onClick={onRetry}>{t("sync.dep.checkAgain")}</LinkButton>
          </div>
        </div>
      </div>
    );
  if (!plan) return null;

  const worker = form.workerName.trim();
  if (plan.worker === "has_vault" || plan.worker === "foreign")
    return (
      <Callout icon="warning" iconColor="var(--orange)">
        {t(plan.worker === "has_vault" ? "sync.dep.plan.hasVault" : "sync.dep.plan.foreign", { name: worker })}
      </Callout>
    );

  const database = plan.database_name ?? "";
  const rows: { icon: string; text: string }[] = [
    {
      icon: plan.worker === "create" ? "plus-circle" : "arrows-clockwise",
      text: t(plan.worker === "create" ? "sync.dep.plan.workerCreate" : "sync.dep.plan.workerUpdate", { name: worker }),
    },
    {
      icon: plan.database === "create" ? "plus-circle" : "database",
      text: t(
        plan.database === "create" ? "sync.dep.plan.dbCreate" : plan.database === "use" ? "sync.dep.plan.dbUse" : "sync.dep.plan.dbBound",
        { name: database },
      ),
    },
  ];
  const renamed = plan.database !== "bound" && database !== form.databaseName.trim();
  return (
    <div className={s.plan}>
      <ul className={s.planList}>
        {rows.map((r) => (
          <li key={r.icon + r.text} className={s.planRow}>
            <Icon name={r.icon} className={s.planIcon} />
            <span>{r.text}</span>
          </li>
        ))}
        {url && (
          <li className={s.planRow}>
            <Icon name="globe-simple" className={s.planIcon} />
            <span className={cx(s.mono, "selectable")}>{url}</span>
          </li>
        )}
      </ul>
      {renamed && <div className={s.planNote}>{t("sync.dep.plan.renamed", { name: form.databaseName.trim(), other: database })}</div>}
    </div>
  );
}

const STEP_ICON: Record<StepState | "pending", { name: string; fill?: boolean; color: string }> = {
  pending: { name: "circle", color: "var(--fg3)" },
  running: { name: "circle", color: "var(--accent)" },
  done: { name: "check-circle", fill: true, color: "var(--green)" },
  skipped: { name: "minus-circle", color: "var(--fg3)" },
  failed: { name: "x-circle", fill: true, color: "var(--red)" },
};

/** The deployment steps with their progress, and the error under the step that failed. */
export function StepList({
  steps,
  states,
  error,
  label,
  ariaLabel,
}: {
  steps: Step[];
  states: Partial<Record<Step, StepState>>;
  error: AppError | null;
  label: (step: Step) => string;
  ariaLabel: string;
}) {
  const t = useT();
  const failedStep = steps.find((step) => states[step] === "failed");
  return (
    <ol className={s.stepList} aria-label={ariaLabel}>
      {steps.map((step) => {
        const state = states[step] ?? "pending";
        const icon = STEP_ICON[state];
        return (
          <li key={step} className={cx(s.stepRow, state === "pending" && s.dim)} aria-current={state === "running" ? "step" : undefined}>
            <span className={s.stepIcon}>
              {state === "running" ? <Spinner /> : <Icon name={icon.name} fill={icon.fill} color={icon.color} size={16} />}
            </span>
            <span className={s.stepText}>
              <span>
                {label(step)}
                {state === "skipped" && <span className={s.dim}> · {t("sync.dep.skipped")}</span>}
              </span>
              {step === failedStep && error && (
                <>
                  <span className={s.stepError} role="alert">
                    {deployError(t, error)}
                  </span>
                  <EditTokenLink error={error} />
                </>
              )}
            </span>
          </li>
        );
      })}
    </ol>
  );
}

function RunPhase({ form, set, handle, onReady }: { form: DeployForm; set: Set; handle: string; onReady: (handle: string) => void }) {
  const t = useT();
  const [checking, setChecking] = useState(false);
  const started = useRef(false);

  const run = async () => {
    set((f) => ({ ...f, run: "running", runError: null, steps: { verify: "done" } }));
    try {
      const outcome = await api.deploy_run(handle, target(form), (p) => set((f) => ({ ...f, steps: { ...f.steps, [p.step]: p.status } })));
      set((f) => ({ ...f, url: outcome.url, run: outcome.ready ? "ready" : "waiting" }));
    } catch (e) {
      set((f) => {
        const current = DEPLOY_STEPS.find((step) => f.steps[step] === "running");
        return { ...f, run: "failed", runError: toAppError(e), steps: current ? { ...f.steps, [current]: "failed" } : f.steps };
      });
    }
  };

  // The review's Deploy button lands here with `run: "idle"`.
  useEffect(() => {
    if (form.run === "idle" && !started.current) {
      started.current = true;
      void run();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const checkAgain = async () => {
    setChecking(true);
    try {
      const ready = await api.deploy_check(handle);
      if (ready) set((f) => ({ ...f, run: "ready", steps: { ...f.steps, wait: "done" } }));
    } catch (e) {
      set((f) => ({ ...f, run: "failed", runError: toAppError(e), steps: { ...f.steps, wait: "failed" } }));
    } finally {
      setChecking(false);
    }
  };

  const remove = async () => {
    set((f) => ({ ...f, run: "removing" }));
    try {
      await api.deploy_cleanup(handle);
      set((f) => ({ ...f, run: "removed", runError: null, steps: {} }));
    } catch (e) {
      set((f) => ({ ...f, run: "failed", runError: toAppError(e) }));
    }
  };

  const toReview = () => set((f) => ({ ...f, phase: "review", run: "idle", plan: null, planError: null, steps: {} }));

  const failedStep = DEPLOY_STEPS.find((step) => form.steps[step] === "failed");
  // Failures that a retry cannot fix send the user back to the review instead.
  const fixInReview = ["remote_initialized", "worker_name_taken", "subdomain_required", "subdomain_unavailable", "invalid_input"].includes(form.runError?.code ?? "");
  const busy = form.run === "running" || form.run === "removing";
  const title =
    form.run === "ready" ? t("sync.dep.run.ready") : form.run === "failed" || form.run === "removed" ? t("sync.dep.run.failed") : t("sync.dep.run.title");

  const footer = (() => {
    switch (form.run) {
      case "ready":
        return (
          <Button variant="primary" onClick={() => onReady(handle)}>
            {t("btn.continue")}
          </Button>
        );
      case "waiting":
        return (
          <Button variant="primary" busy={checking} onClick={() => void checkAgain()}>
            {t("sync.dep.checkAgain")}
          </Button>
        );
      case "failed":
        return fixInReview ? (
          <Button variant="primary" onClick={toReview}>
            {t("btn.back")}
          </Button>
        ) : (
          <>
            <Button onClick={toReview}>{t("btn.back")}</Button>
            <Button variant="primary" onClick={() => void run()}>
              {t("btn.retry")}
            </Button>
          </>
        );
      case "removed":
        return (
          <>
            <Button onClick={toReview}>{t("btn.back")}</Button>
            <Button variant="primary" onClick={() => void run()}>
              {t("sync.dep.deploy")}
            </Button>
          </>
        );
      default:
        return (
          <Button variant="primary" disabled busy={busy}>
            {t("btn.continue")}
          </Button>
        );
    }
  })();

  return (
    <WizardFrame
      step={2}
      footerLead={
        form.run === "failed" && !fixInReview ? (
          <LinkButton tone="danger" onClick={() => void remove()}>
            {t("sync.dep.remove")}
          </LinkButton>
        ) : form.run === "removing" ? (
          <>
            <Spinner />
            {t("sync.dep.removing")}
          </>
        ) : undefined
      }
      footer={footer}
    >
      <WizardTitle title={title} />
      <StepList
        steps={DEPLOY_STEPS}
        states={form.steps}
        error={form.runError}
        label={(step) => t(`sync.dep.step.${step}` as MessageKey)}
        ariaLabel={t("sync.dep.run.title")}
      />
      {form.run === "failed" && !fixInReview && <div className={s.planNote}>{t("sync.dep.retryHint")}</div>}
      {form.run === "failed" && !failedStep && form.runError && (
        <div className={s.stepError} role="alert">
          {deployError(t, form.runError)}
        </div>
      )}
      {form.run === "waiting" && <Callout icon="hourglass-medium" iconColor="var(--orange)">{t("sync.dep.waiting")}</Callout>}
      {form.run === "removed" && <Callout icon="trash">{t("sync.dep.removed")}</Callout>}
      {form.run === "ready" && form.url && (
        <Callout icon="check-circle" iconColor="var(--green)">
          {t("sync.dep.ready")}
          <div className={cx(s.mono, "selectable")}>{form.url}</div>
        </Callout>
      )}
    </WizardFrame>
  );
}
