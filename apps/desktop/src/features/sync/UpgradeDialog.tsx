import { useEffect, useId, useRef, useState } from "react";
import { Button, Icon, LinkButton, Spinner, TextField } from "@/components/controls";
import { Callout } from "@/components/layout";
import { Dialog, PopupSelect } from "@/components/overlay";
import { Field } from "@/features/keys/Field";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { AppError, DeployStart, DeployStep, UpgradeDefaults, UpgradePlan } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { deployError, EditTokenLink, LABEL, StepList } from "./DeployStep";
import { DEPLOY_GUIDE_URL, DEPLOY_TOKEN_URL, openExternal } from "./external";
import type { T } from "./syncUtils";
import type { StepState } from "./wizardTypes";
import s from "./Wizard.module.css";
import d from "./Dialogs.module.css";

/** Steps 1 to 8 without 3 and 6, which an upgrade never runs. */
const UPGRADE_STEPS: DeployStep[] = ["verify", "inspect", "migrate", "upload", "route", "wait"];

const UPGRADE_GUIDE_URL = `${DEPLOY_GUIDE_URL}#upgrade`;

/** Failures a retry cannot fix: the review shows what step 2 found instead. */
const FIX_IN_REVIEW = ["worker_not_found", "worker_newer", "invalid_input"];

/** What step 2 finds when the name is not the Worker's: the review then asks for it. */
const WRONG_NAME: UpgradePlan["worker"][] = ["missing", "foreign", "no_vault"];

const keyOf = (account: string, worker: string) => `${account}|${worker}`;

type Run = "running" | "failed" | "waiting" | "ready";

const stepLabel = (t: T) => (step: DeployStep) =>
  t(step === "inspect" || step === "migrate" ? `sync.up.step.${step}` : (`sync.dep.step.${step}` as MessageKey));

/**
 * "Update Worker" (spec §6.7, Upgrades): an API token, a review of what step 2 found, then the
 * deployment steps as an upgrade. The token goes to Rust once, and closing the dialog ends the
 * deployment, which wipes it (DEPLOY-07).
 */
export function UpgradeDialog({ bundled, onClose }: { bundled: string; onClose: () => void }) {
  const t = useT();
  const tokenId = useId();
  const accountFieldId = useId();
  const workerId = useId();
  const formId = useId();
  const [defaults, setDefaults] = useState<UpgradeDefaults | null>(null);
  const [phase, setPhase] = useState<"token" | "review" | "run">("token");
  const [apiToken, setApiToken] = useState("");
  const [accountId, setAccountId] = useState("");
  const [workerName, setWorkerName] = useState("");
  const [start, setStart] = useState<DeployStart | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  /** Step 2's plan, with the account and name it checked. */
  const [checked, setChecked] = useState<{ key: string; plan: UpgradePlan } | null>(null);
  const [inspecting, setInspecting] = useState(false);
  /** Neither the deployment nor the token gave an account ID, so the review asks for it. */
  const [enterAccount, setEnterAccount] = useState(false);
  /** Step 2 found no Worker to update under the name, so the review asks for it. */
  const [askName, setAskName] = useState(false);
  const [steps, setSteps] = useState<Partial<Record<DeployStep, StepState>>>({});
  const [run, setRun] = useState<Run>("running");
  const [runError, setRunError] = useState<AppError | null>(null);
  const handle = useRef<string | null>(null);
  const request = useRef(0);
  /** The account and name step 2 last looked at, so Enter and the blur after it ask once. */
  const inspected = useRef("");
  /** The upgrade started, so the Worker may have changed by the time the dialog closes. */
  const ran = useRef(false);

  useEffect(() => {
    api.deploy_upgrade_defaults().then(
      (found) => {
        setDefaults(found);
        setAccountId(found.account_id ?? "");
        setWorkerName(found.worker_name ?? "");
      },
      (e) => setError(toAppError(e)),
    );
    return () => {
      if (handle.current) void api.deploy_cancel(handle.current);
      // After a close while waiting or after a failure, the sync status page would keep the
      // notice for the old version.
      if (ran.current) void api.sync_check_worker().catch(() => {});
    };
  }, []);

  const name = workerName.trim();
  const nameOk = LABEL.test(name);
  const target = () => ({ account_id: accountId.trim(), worker_name: name });
  // Only a plan for the account and name in the form counts.
  const plan = checked?.key === keyOf(accountId.trim(), name) ? checked.plan : null;

  const inspect = async (account = accountId.trim(), worker = name, again = false) => {
    const key = keyOf(account, worker);
    if (!handle.current || !account || !LABEL.test(worker) || (key === inspected.current && !again)) return;
    inspected.current = key;
    const n = ++request.current;
    setInspecting(true);
    setError(null);
    setChecked(null);
    try {
      const found = await api.deploy_upgrade_inspect(handle.current, { account_id: account, worker_name: worker });
      if (n === request.current) {
        setChecked({ key, plan: found });
        if (WRONG_NAME.includes(found.worker)) setAskName(true);
      }
    } catch (e) {
      if (n === request.current) setError(toAppError(e));
    } finally {
      if (n === request.current) setInspecting(false);
    }
  };

  const verify = async () => {
    if (!apiToken.trim() || !nameOk || busy) return;
    setBusy(true);
    setError(null);
    try {
      const started = await api.deploy_start(apiToken.trim(), accountId.trim() || null);
      handle.current = started.handle;
      // DEPLOY-07: Rust has the token now; the WebView forgets it.
      setApiToken("");
      setStart(started);
      const account = accountId.trim() || (started.accounts.length === 1 ? started.accounts[0].id : "");
      setAccountId(account);
      setEnterAccount(!account);
      setPhase("review");
      void inspect(account);
    } catch (e) {
      setError(toAppError(e));
    } finally {
      setBusy(false);
    }
  };

  const back = () => {
    if (handle.current) void api.deploy_cancel(handle.current);
    handle.current = null;
    inspected.current = "";
    // A check still running answers for the deployment that just ended.
    request.current++;
    setInspecting(false);
    setStart(null);
    setChecked(null);
    setError(null);
    setPhase("token");
  };

  const upgrade = async () => {
    if (!handle.current) return;
    ran.current = true;
    setPhase("run");
    setRun("running");
    setRunError(null);
    setSteps({ verify: "done" });
    try {
      const outcome = await api.deploy_upgrade(handle.current, target(), (p) => setSteps((st) => ({ ...st, [p.step]: p.status })));
      setRun(outcome.ready ? "ready" : "waiting");
    } catch (e) {
      setSteps((st) => {
        const current = UPGRADE_STEPS.find((step) => st[step] === "running");
        return current ? { ...st, [current]: "failed" } : st;
      });
      setRunError(toAppError(e));
      setRun("failed");
    }
  };

  const checkAgain = async () => {
    if (!handle.current) return;
    setBusy(true);
    try {
      if (await api.deploy_check(handle.current)) {
        setSteps((st) => ({ ...st, wait: "done" }));
        setRun("ready");
      }
    } catch (e) {
      setSteps((st) => ({ ...st, wait: "failed" }));
      setRunError(toAppError(e));
      setRun("failed");
    } finally {
      setBusy(false);
    }
  };

  const toReview = () => {
    setPhase("review");
    void inspect(accountId.trim(), name, true);
  };

  const running = phase === "run" && run === "running";
  const closable = !busy && !running;

  if (phase === "run") {
    const fixInReview = FIX_IN_REVIEW.includes(runError?.code ?? "");
    const failedStep = UPGRADE_STEPS.find((step) => steps[step] === "failed");
    const title = run === "ready" ? t("sync.up.run.ready") : run === "failed" ? t("sync.up.run.failed") : t("sync.up.run.title");
    return (
      <Dialog
        title={title}
        icon={run === "ready" ? "check-circle" : run === "failed" ? "warning-circle" : "arrows-clockwise"}
        iconColor={run === "ready" ? "var(--green)" : run === "failed" ? "var(--orange)" : undefined}
        onClose={closable ? onClose : undefined}
        actions={
          run === "ready" ? (
            <Button variant="primary" onClick={onClose} autoFocus>
              {t("btn.done")}
            </Button>
          ) : run === "waiting" ? (
            <>
              <Button onClick={onClose} disabled={busy}>
                {t("btn.close")}
              </Button>
              <Button variant="primary" busy={busy} onClick={() => void checkAgain()}>
                {t("sync.dep.checkAgain")}
              </Button>
            </>
          ) : run === "failed" ? (
            <>
              <Button onClick={fixInReview ? toReview : onClose}>{t(fixInReview ? "btn.back" : "btn.close")}</Button>
              {!fixInReview && (
                <Button variant="primary" onClick={() => void upgrade()}>
                  {t("btn.retry")}
                </Button>
              )}
            </>
          ) : (
            <Button variant="primary" disabled busy>
              {t("sync.up.update")}
            </Button>
          )
        }
      >
        <div className={d.form}>
          <StepList steps={UPGRADE_STEPS} states={steps} error={runError} label={stepLabel(t)} ariaLabel={t("sync.up.run.title")} />
          {run === "failed" && !failedStep && runError && (
            <div className={s.stepError} role="alert">
              {deployError(t, runError)}
            </div>
          )}
          {run === "failed" && !fixInReview && <div className={s.planNote}>{t("sync.up.retryHint")}</div>}
          {run === "waiting" && (
            <Callout icon="hourglass-medium" iconColor="var(--orange)">
              {t("sync.up.waiting")}
            </Callout>
          )}
          {run === "ready" && (
            <Callout icon="check-circle" iconColor="var(--green)">
              {t("sync.up.ready", { version: bundled })}
            </Callout>
          )}
        </div>
      </Dialog>
    );
  }

  if (phase === "review" && start) {
    const canUpgrade = plan?.worker === "upgrade" && !inspecting;
    const accountName = start.accounts.find((a) => a.id === accountId)?.name;
    return (
      <Dialog
        title={t("sync.up.review.title")}
        icon="arrow-circle-up"
        body={t("sync.up.review.body")}
        onClose={onClose}
        actions={
          <>
            <Button onClick={back}>{t("btn.back")}</Button>
            <Button variant="primary" disabled={!canUpgrade} onClick={() => void upgrade()}>
              {t("sync.up.update")}
            </Button>
          </>
        }
      >
        <div className={d.form}>
          <Field label={t("sync.dep.account")} htmlFor={accountFieldId}>
            {start.accounts.length > 1 ? (
              <PopupSelect
                ariaLabel={t("sync.dep.account")}
                icon="buildings"
                value={accountId}
                placeholder={t("sync.dep.account.choose")}
                options={start.accounts.map((a) => ({ value: a.id, label: a.name || a.id, hint: a.name ? a.id.slice(0, 8) : undefined }))}
                onChange={(id) => {
                  setAccountId(id);
                  void inspect(id);
                }}
              />
            ) : enterAccount ? (
              <TextField
                id={accountFieldId}
                mono
                value={accountId}
                placeholder={t("sync.dep.accountId.enter")}
                onChange={(e) => setAccountId(e.target.value)}
                onBlur={() => void inspect()}
                onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void inspect()}
              />
            ) : (
              <div className={s.accountLine}>
                {accountName && <span>{accountName}</span>}
                <span className={cx(s.mono, s.dim)}>{accountId}</span>
              </div>
            )}
          </Field>
          <UpgradeSummary t={t} name={name} plan={plan} inspecting={inspecting} error={error} onRetry={() => void inspect(accountId.trim(), name, true)} />
          {(askName || !defaults?.worker_name) && (
            <Field
              label={t("sync.dep.workerName")}
              htmlFor={workerId}
              error={workerName && !nameOk ? t("sync.dep.invalid.worker_name") : undefined}
            >
              <TextField
                id={workerId}
                mono
                value={workerName}
                invalid={!!workerName && !nameOk}
                onChange={(e) => setWorkerName(e.target.value.toLowerCase())}
                onBlur={() => void inspect()}
                onKeyDown={(e) => e.key === "Enter" && !isImeEvent(e) && void inspect()}
              />
            </Field>
          )}
        </div>
      </Dialog>
    );
  }

  return (
    <Dialog
      title={t("sync.up.title")}
      icon="arrow-circle-up"
      body={t("sync.up.body", { version: bundled })}
      onClose={closable ? onClose : undefined}
      actions={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy} disabled={!defaults || !apiToken.trim() || !nameOk}>
            {t("btn.continue")}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className={d.form}
        onSubmit={(e) => {
          e.preventDefault();
          void verify();
        }}
      >
        {defaults && !defaults.deployed_by_app && (
          <Callout icon="warning" iconColor="var(--orange)">
            {t("sync.up.buttonWarning")}
            <div className={s.calloutLink}>
              <LinkButton onClick={() => void openExternal(UPGRADE_GUIDE_URL)}>
                <span className={s.externalLink}>
                  {t("sync.up.guide")}
                  <Icon name="arrow-up-right" />
                </span>
              </LinkButton>
            </div>
          </Callout>
        )}
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
        <Field label={t("sync.apiToken")} htmlFor={tokenId} error={error ? deployError(t, error) : undefined} hint={t("sync.dep.token.memory")}>
          <TextField
            id={tokenId}
            secret
            autoFocus
            autoComplete="off"
            value={apiToken}
            disabled={busy}
            invalid={!!error}
            onChange={(e) => {
              setApiToken(e.target.value);
              setError(null);
            }}
          />
        </Field>
        <EditTokenLink error={error} />
        <Field label={t("sync.dep.accountId")} htmlFor={accountFieldId} hint={t("sync.dep.accountId.hint")}>
          <TextField
            id={accountFieldId}
            mono
            value={accountId}
            disabled={busy}
            onChange={(e) => {
              setAccountId(e.target.value);
              setError(null);
            }}
          />
        </Field>
        {defaults && !defaults.worker_name && (
          <Field
            label={t("sync.dep.workerName")}
            htmlFor={workerId}
            hint={t("sync.up.workerName.hint")}
            error={workerName && !nameOk ? t("sync.dep.invalid.worker_name") : undefined}
          >
            <TextField
              id={workerId}
              mono
              value={workerName}
              disabled={busy}
              invalid={!!workerName && !nameOk}
              onChange={(e) => setWorkerName(e.target.value.toLowerCase())}
            />
          </Field>
        )}
      </form>
    </Dialog>
  );
}

function UpgradeSummary({
  t,
  name,
  plan,
  inspecting,
  error,
  onRetry,
}: {
  t: T;
  name: string;
  plan: UpgradePlan | null;
  inspecting: boolean;
  error: AppError | null;
  onRetry: () => void;
}) {
  if (inspecting)
    return (
      <div className={s.planStatus} role="status">
        <Spinner />
        {t("sync.dep.checking")}
      </div>
    );
  if (error)
    return (
      <div className={s.tokenError} role="alert">
        <Icon name="warning-circle" />
        <div className={s.tokenErrorText}>
          <div>{deployError(t, error)}</div>
          <div className={s.linkRow}>
            <EditTokenLink error={error} />
            <LinkButton onClick={onRetry}>{t("sync.dep.checkAgain")}</LinkButton>
          </div>
        </div>
      </div>
    );
  if (!plan) return null;

  if (plan.worker !== "upgrade") {
    const key = ({ missing: "sync.up.plan.missing", foreign: "sync.up.plan.foreign", no_vault: "sync.up.plan.noVault", newer: "sync.up.plan.newer" } as const)[
      plan.worker
    ];
    return (
      <Callout icon="warning" iconColor="var(--orange)">
        {t(key, { name, version: plan.version ?? "", bundled: plan.bundled })}
      </Callout>
    );
  }

  const database = plan.database_name ?? "";
  const rows = [
    {
      icon: "arrow-circle-up",
      text: plan.version
        ? t("sync.up.plan.worker", { name, from: plan.version, to: plan.bundled })
        : t("sync.up.plan.workerTo", { name, to: plan.bundled }),
    },
    {
      icon: "database",
      text:
        plan.migrations > 0
          ? t("sync.up.plan.migrations", { n: plan.migrations, name: database })
          : t("sync.up.plan.noMigrations", { name: database }),
    },
  ];
  return (
    <div className={s.plan}>
      <ul className={s.planList}>
        {rows.map((r) => (
          <li key={r.icon} className={s.planRow}>
            <Icon name={r.icon} className={s.planIcon} />
            <span>{r.text}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}
