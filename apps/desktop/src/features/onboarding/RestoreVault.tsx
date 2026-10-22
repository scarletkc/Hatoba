import { useState } from "react";
import { enterUnlocked } from "@/app/boot";
import { errorMessage } from "@/app/errors";
import { Button, Icon, Segmented, TextField } from "@/components/controls";
import { FooterSpacer } from "@/components/overlay";
import { ErrorText, StatusLine } from "@/components/patterns";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { SyncConfigInput } from "@/ipc/types";
import { WizardCard } from "./WizardCard";
import s from "./RestoreVault.module.css";

type Method = "worker" | "d1";
type TestStatus = { state: "idle" } | { state: "running" } | { state: "ok" | "warn" | "error"; text: string };

/** Accepts "host.workers.dev" as well as full URLs; returns null when it can't be an http(s) URL. */
function normalizeWorkerUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (!trimmed) return null;
  const withScheme = /^https?:\/\//i.test(trimmed) ? trimmed : `https://${trimmed}`;
  try {
    const url = new URL(withScheme);
    if (!url.hostname.includes(".") && url.hostname !== "localhost") return null;
    return url.origin + url.pathname.replace(/\/+$/, "");
  } catch {
    return null;
  }
}

/** Flow B step 2: where the cloud vault lives, with a connection test (`initialized` must be true). */
export function RestoreConnect({
  steps,
  onBack,
  onConnected,
}: {
  steps: string[];
  onBack: () => void;
  onConnected: (config: SyncConfigInput) => void;
}) {
  const t = useT();
  const [method, setMethod] = useState<Method>("worker");
  const [url, setUrl] = useState("");
  const [accountId, setAccountId] = useState("");
  const [databaseId, setDatabaseId] = useState("");
  const [apiToken, setApiToken] = useState("");
  const [status, setStatus] = useState<TestStatus>({ state: "idle" });
  const [urlError, setUrlError] = useState<string | null>(null);

  const reset = () => {
    setStatus({ state: "idle" });
    setUrlError(null);
  };

  const buildConfig = (): SyncConfigInput | null => {
    if (method === "worker") {
      const normalized = normalizeWorkerUrl(url);
      if (!normalized) {
        setUrlError(t("vault.restore.badUrl"));
        return null;
      }
      return { kind: "worker", url: normalized, setup_token: null };
    }
    if (!accountId.trim() || !databaseId.trim() || !apiToken.trim()) return null;
    return { kind: "d1", account_id: accountId.trim(), database_id: databaseId.trim(), api_token: apiToken.trim() };
  };

  const filled = method === "worker" ? url.trim() !== "" : accountId.trim() !== "" && databaseId.trim() !== "" && apiToken.trim() !== "";
  const ready = status.state === "ok";

  const test = async () => {
    const config = buildConfig();
    if (!config) return;
    setStatus({ state: "running" });
    try {
      const r = await api.sync_test(config);
      if (!r.ok) {
        setStatus({ state: "error", text: errorMessage(t, r.error ?? { code: "sync", detail: "" }) });
      } else if (!r.initialized) {
        setStatus({ state: "warn", text: t("err.remote_not_initialized") });
      } else {
        const base = t("vault.restore.found");
        setStatus({ state: "ok", text: r.latency_ms !== null ? `${base} · ${r.latency_ms} ms` : base });
      }
    } catch (e) {
      setStatus({ state: "error", text: errorMessage(t, e) });
    }
  };

  // Enter tests first, then continues once the test passed.
  const submit = () => {
    if (!ready) return void test();
    const config = buildConfig();
    if (config) onConnected(config);
  };

  return (
    <WizardCard
      steps={steps}
      current={2}
      title={t("vault.restore.title")}
      body={t("vault.restore.body")}
      onSubmit={submit}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onBack}>{t("btn.back")}</Button>
          <Button variant="primary" type="submit" disabled={!ready}>
            {t("btn.continue")}
          </Button>
        </>
      }
    >
      <div className={s.field}>
        <span className={s.label}>{t("vault.restore.method")}</span>
        <Segmented
          ariaLabel={t("vault.restore.method")}
          value={method}
          onChange={(m) => {
            setMethod(m);
            reset();
          }}
          options={[
            { value: "worker", label: t("vault.restore.worker"), icon: "cloud-arrow-up" },
            { value: "d1", label: t("vault.restore.d1"), icon: "database" },
          ]}
        />
      </div>

      {method === "worker" ? (
        <div className={s.field}>
          <label className={s.label} htmlFor="restore-url">
            {t("vault.restore.workerUrl")}
          </label>
          <TextField
            id="restore-url"
            large
            mono
            autoFocus
            inputMode="url"
            placeholder={t("vault.restore.workerUrlPlaceholder")}
            value={url}
            invalid={!!urlError}
            trailing={status.state === "ok" ? <Icon name="check-circle" fill size={16} color="var(--green)" /> : undefined}
            onChange={(e) => {
              setUrl(e.target.value);
              reset();
            }}
          />
          {urlError && <ErrorText>{urlError}</ErrorText>}
        </div>
      ) : (
        <>
          <div className={s.field}>
            <label className={s.label} htmlFor="restore-account">
              {t("vault.restore.accountId")}
            </label>
            <TextField id="restore-account" large mono autoFocus value={accountId} onChange={(e) => (setAccountId(e.target.value), reset())} />
          </div>
          <div className={s.field}>
            <label className={s.label} htmlFor="restore-database">
              {t("vault.restore.databaseId")}
            </label>
            <TextField id="restore-database" large mono value={databaseId} onChange={(e) => (setDatabaseId(e.target.value), reset())} />
          </div>
          <div className={s.field}>
            <label className={s.label} htmlFor="restore-token">
              {t("vault.restore.apiToken")}
            </label>
            <TextField id="restore-token" large secret value={apiToken} onChange={(e) => (setApiToken(e.target.value), reset())} />
          </div>
        </>
      )}

      <div className={s.testRow}>
        <Button icon="plugs-connected" disabled={!filled} busy={status.state === "running"} onClick={() => void test()}>
          {t("vault.restore.test")}
        </Button>
        <div className={s.status} role="status">
          {status.state === "running" && <StatusLine tone="progress">{t("vault.restore.testing")}</StatusLine>}
          {(status.state === "ok" || status.state === "warn" || status.state === "error") && (
            <StatusLine tone={status.state === "ok" ? "success" : status.state === "warn" ? "warning" : "error"}>{status.text}</StatusLine>
          )}
        </div>
      </div>
    </WizardCard>
  );
}

/** Flow B step 3: master password, then pull and decrypt everything. */
export function RestorePassword({ steps, config, onBack }: { steps: string[]; config: SyncConfigInput; onBack: () => void }) {
  const t = useT();
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      await api.vault_restore_from_cloud(config, password);
      setPassword("");
      await enterUnlocked();
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  };

  return (
    <WizardCard
      steps={steps}
      current={3}
      title={t("vault.field.master")}
      body={t("vault.restore.passwordHint")}
      onSubmit={() => void submit()}
      footer={
        <>
          {busy && <span className={s.busyText}>{t("vault.restore.busy")}</span>}
          <FooterSpacer />
          <Button onClick={onBack} disabled={busy}>
            {t("btn.back")}
          </Button>
          <Button variant="primary" type="submit" busy={busy} disabled={!password}>
            {t("vault.restore.submit")}
          </Button>
        </>
      }
    >
      <div className={s.field}>
        <label className={s.label} htmlFor="restore-password">
          {t("vault.field.master")}
        </label>
        <TextField
          id="restore-password"
          large
          secret
          autoFocus
          autoComplete="current-password"
          readOnly={busy}
          invalid={!!error}
          value={password}
          onChange={(e) => {
            setPassword(e.target.value);
            setError(null);
          }}
        />
      </div>
      {error && <ErrorText icon="warning-circle">{error}</ErrorText>}
    </WizardCard>
  );
}
