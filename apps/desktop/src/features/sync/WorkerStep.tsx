import { useId, useState } from "react";
import { Button, Icon, LinkButton, StatusDot, TextField } from "@/components/controls";
import { errorMessage } from "@/app/errors";
import { Field } from "@/features/keys/Field";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { SyncConfigInput } from "@/ipc/types";
import { DEPLOY_GUIDE_URL, DEPLOY_URL, openExternal } from "./external";
import { normalizeWorkerUrl, type WorkerForm } from "./wizardTypes";
import { WizardFrame, WizardTitle } from "./WizardFrame";
import s from "./Wizard.module.css";

/** The manual steps from the Worker deployment guide (`workers/sync/README.md`), in its order. */
const WRANGLER_STEPS = [
  "npm install",
  "npx wrangler d1 create hatoba",
  "npx wrangler d1 migrations apply hatoba --remote",
  "npx wrangler secret put SETUP_TOKEN",
  "npx wrangler deploy",
];

/** Step 2a: deploy the Worker, then test URL + Setup Token. */
export function WorkerStep({
  form,
  update,
  onBack,
  onNext,
}: {
  form: WorkerForm;
  update: (patch: Partial<WorkerForm>) => void;
  onBack: () => void;
  onNext: (config: SyncConfigInput) => void;
}) {
  const t = useT();
  const urlId = useId();
  const tokenId = useId();
  const [wranglerOpen, setWranglerOpen] = useState(true);
  const { test } = form;
  const ok = test.status === "ok" ? test.result : null;
  const initialized = !!ok?.initialized;
  const canTest = form.url.trim().length > 0 && test.status !== "testing";
  const canContinue = !!ok && !initialized && form.token.trim().length > 0;

  const runTest = async () => {
    if (!canTest) return;
    const url = normalizeWorkerUrl(form.url);
    update({ url, test: { status: "testing" } });
    try {
      const result = await api.sync_test({ kind: "worker", url, setup_token: form.token.trim() || null });
      update({
        test: result.ok
          ? { status: "ok", result }
          : { status: "failed", message: result.error ? errorMessage(t, result.error) : t("sync.test.failed") },
      });
    } catch (e) {
      update({ test: { status: "failed", message: errorMessage(t, e) } });
    }
  };

  const status = (() => {
    if (test.status === "ok") {
      if (initialized) return { color: "var(--orange)", text: t("err.remote_initialized") };
      const parts = [t("sync.test.connected")];
      if (test.result.version) parts.push(t("sync.test.version", { version: test.result.version }));
      if (test.result.latency_ms !== null) parts.push(`${test.result.latency_ms} ms`);
      return { color: "var(--green)", text: parts.join(" · ") };
    }
    if (test.status === "failed") return { color: "var(--red)", text: test.message };
    return null;
  })();

  return (
    <WizardFrame
      step={2}
      footer={
        <>
          <Button onClick={onBack}>{t("btn.back")}</Button>
          <Button variant="primary" disabled={!canContinue} onClick={() => ok && onNext({ kind: "worker", url: normalizeWorkerUrl(form.url), setup_token: form.token.trim() })}>
            {t("btn.continue")}
          </Button>
        </>
      }
    >
      <WizardTitle title={t("sync.w2.title")} body={t("sync.w2.body")} />

      <div className={s.oneClick}>
        <Icon name="cloud" className={s.oneClickIcon} />
        <div className={s.oneClickText}>
          <div className={s.oneClickTitle}>{t("sync.oneClick")}</div>
          <div className={s.oneClickDesc}>{t("sync.oneClick.desc")}</div>
        </div>
        <Button trailingIcon="arrow-up-right" onClick={() => void openExternal(DEPLOY_URL)}>
          {t("sync.deploy")}
        </Button>
      </div>

      <div className={s.wrangler}>
        <button type="button" className={s.disclosure} aria-expanded={wranglerOpen} onClick={() => setWranglerOpen((v) => !v)}>
          <Icon name={wranglerOpen ? "caret-down" : "caret-right"} />
          {t("sync.wrangler")}
        </button>
        {wranglerOpen && (
          <>
            <div className={s.cwd}>{t("sync.wrangler.cwd")}</div>
            <div className={`${s.code} selectable`}>
              {WRANGLER_STEPS.map((line) => (
                <div key={line} className={s.codeLine}>
                  <span className={s.prompt}>$ </span>
                  {line}
                </div>
              ))}
            </div>
            <LinkButton className={s.guideLink} onClick={() => void openExternal(DEPLOY_GUIDE_URL)}>
              <span className={s.externalLink}>
                {t("sync.wrangler.guide")}
                <Icon name="arrow-up-right" />
              </span>
            </LinkButton>
          </>
        )}
      </div>

      <div className={s.fields}>
        <Field label={t("sync.workerUrl")} htmlFor={urlId}>
          <TextField
            id={urlId}
            mono
            large
            value={form.url}
            placeholder={t("sync.workerUrl.placeholder")}
            inputMode="url"
            invalid={test.status === "failed"}
            trailing={ok && !initialized ? <Icon name="check-circle" fill size={15} color="var(--green)" /> : undefined}
            onChange={(e) => update({ url: e.target.value, test: { status: "idle" } })}
            onKeyDown={(e) => e.key === "Enter" && void runTest()}
          />
        </Field>
        <Field label={t("sync.setupToken")} htmlFor={tokenId} hint={t("sync.setupToken.hint")}>
          <TextField
            id={tokenId}
            large
            secret
            value={form.token}
            onChange={(e) => update({ token: e.target.value })}
            onKeyDown={(e) => e.key === "Enter" && void runTest()}
          />
        </Field>
        <div className={s.testRow}>
          <Button size="sm" icon="plugs-connected" busy={test.status === "testing"} disabled={!canTest} onClick={() => void runTest()}>
            {t("sync.test")}
          </Button>
          {status && (
            <div className={s.testStatus} role="status">
              <span className={s.dotSlot}>
                <StatusDot color={status.color} />
              </span>
              <span>{status.text}</span>
            </div>
          )}
        </div>
      </div>
    </WizardFrame>
  );
}
