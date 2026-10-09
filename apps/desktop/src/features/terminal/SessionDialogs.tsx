import { useCallback, useEffect, useState, type FormEvent } from "react";
import { Button, Icon, TextField } from "@/components/controls";
import { Modal } from "@/components/overlay";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { startSessionListeners } from "./listeners";
import {
  answerAuth,
  answerHostKey,
  answerSecret,
  answerUsername,
  usePrompts,
  type PromptItem,
} from "./prompts";
import s from "./SessionDialogs.module.css";

/** Global host-key / keyboard-interactive / one-off secret dialogs (mounted once by App). */
export function SessionDialogs() {
  const item = usePrompts((st) => st.items[0]);
  // Backend events are wired once, after boot, and stay wired while the vault is locked.
  useEffect(() => startSessionListeners(), []);
  if (!item) return null;
  switch (item.kind) {
    case "hostkey":
      return item.prompt.kind === "changed" ? (
        <HostKeyChanged key={item.key} item={item as HostKeyItem} />
      ) : (
        <HostKeyNew key={item.key} item={item as HostKeyItem} />
      );
    case "auth":
      return <AuthPromptDialog key={item.key} item={item} />;
    case "secret":
      return <SecretDialog key={item.key} item={item} />;
    case "username":
      return <UsernameDialog key={item.key} item={item} />;
  }
}

type HostKeyItem = PromptItem & { kind: "hostkey" };

/** SSH-04: first connection (trust on first use). */
function HostKeyNew({ item }: { item: HostKeyItem }) {
  const t = useT();
  const p = item.prompt;
  const reject = useCallback(() => answerHostKey(item, false), [item]);
  return (
    <Modal center onClose={reject} closeOnBackdrop={false}>
      <div role="alertdialog" aria-modal aria-labelledby="hk-title" className={s.card}>
        <div className={s.head}>
          <Icon name="shield-check" size={20} color="var(--accent)" />
          <div id="hk-title" className={s.title}>
            {t("terminal.hostkey.newTitle", { host: p.host, port: p.port })}
          </div>
        </div>
        <div className={s.body}>{t("terminal.hostkey.newBody")}</div>
        <Fingerprint label={t("terminal.hostkey.fingerprint")} keyType={p.key_type} value={p.fingerprint} />
        <div className={s.actions}>
          <Button onClick={reject}>{t("btn.cancel")}</Button>
          <Button variant="primary" onClick={() => answerHostKey(item, true)}>
            {t("terminal.hostkey.trust")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

/**
 * SSH-04: the host key differs from the saved one. The connection is blocked; disconnecting is the
 * default (first, focused) action and updating the fingerprint is an explicit, danger-styled choice.
 */
function HostKeyChanged({ item }: { item: HostKeyItem }) {
  const t = useT();
  const p = item.prompt;
  const reject = useCallback(() => answerHostKey(item, false), [item]);
  return (
    <Modal center onClose={reject} closeOnBackdrop={false}>
      <div role="alertdialog" aria-modal aria-labelledby="hk-title" className={cx(s.card, s.cardDanger)}>
        <div className={s.head}>
          <Icon name="warning-octagon" fill size={24} color="var(--red)" />
          <div id="hk-title" className={cx(s.title, s.titleDanger)}>
            {t("terminal.hostkey.changedTitle")}
          </div>
        </div>
        <div className={s.body}>{t("terminal.hostkey.changedBody", { host: p.host, port: p.port })}</div>
        <Fingerprint
          label={t("terminal.hostkey.known")}
          keyType={p.known_key_type ?? p.key_type}
          value={p.known_fingerprint ?? "—"}
        />
        <Fingerprint label={t("terminal.hostkey.received")} keyType={p.key_type} value={p.fingerprint} danger />
        <div className={s.actions}>
          <Button variant="primary" autoFocus onClick={reject}>
            {t("terminal.hostkey.abort")}
          </Button>
          <Button variant="danger" onClick={() => answerHostKey(item, true)}>
            {t("terminal.hostkey.update")}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

function Fingerprint({ label, keyType, value, danger }: { label: string; keyType: string; value: string; danger?: boolean }) {
  return (
    <div className={cx(s.fp, danger && s.fpDanger)}>
      <div className={s.fpLabel}>
        <span>{label}</span>
        <span className={s.fpType}>{keyType}</span>
      </div>
      <div className={cx(s.fpValue, "selectable")}>{value}</div>
    </div>
  );
}

/** SSH-08: keyboard-interactive / OTP prompts, one input per prompt. */
function AuthPromptDialog({ item }: { item: PromptItem & { kind: "auth" } }) {
  const t = useT();
  const p = item.prompt;
  const [answers, setAnswers] = useState<string[]>(() => p.prompts.map(() => ""));
  const cancel = useCallback(() => answerAuth(item, null), [item]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    answerAuth(item, answers);
  };
  return (
    <Modal center onClose={cancel} closeOnBackdrop={false}>
      <form role="dialog" aria-modal aria-labelledby="auth-title" className={s.card} onSubmit={submit}>
        <div className={s.head}>
          <Icon name="password" size={20} color="var(--accent)" />
          <div id="auth-title" className={s.title}>
            {p.name.trim() ? p.name : t("terminal.auth.title")}
          </div>
        </div>
        {p.instructions.trim() && <div className={cx(s.body, s.instructions)}>{p.instructions}</div>}
        <div className={s.fields}>
          {p.prompts.map((q, i) => (
            <label key={i} className={s.field}>
              <span className={s.fieldLabel}>{q.prompt}</span>
              <TextField
                secret={!q.echo}
                mono
                large
                autoFocus={i === 0}
                value={answers[i]}
                onChange={(e) => setAnswers((a) => a.map((v, j) => (j === i ? e.target.value : v)))}
              />
            </label>
          ))}
        </div>
        <div className={s.actions}>
          <Button onClick={cancel}>{t("btn.cancel")}</Button>
          <Button variant="primary" type="submit">
            {t("terminal.auth.submit")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

/** SSH-03 / key passphrase: a secret used once and never stored. */
function SecretDialog({ item }: { item: PromptItem & { kind: "secret" } }) {
  const t = useT();
  const r = item.request;
  const [value, setValue] = useState("");
  const isPassword = r.kind === "password";
  const cancel = useCallback(() => answerSecret(item, null), [item]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (value) answerSecret(item, value);
  };
  return (
    <Modal center onClose={cancel} closeOnBackdrop={false}>
      <form role="dialog" aria-modal aria-labelledby="secret-title" className={s.card} onSubmit={submit}>
        <div className={s.head}>
          <Icon name={isPassword ? "lock-key" : "key"} size={20} color="var(--accent)" />
          <div id="secret-title" className={s.title}>
            {isPassword
              ? t("terminal.password.title", { name: r.hostName })
              : t("terminal.passphrase.title")}
          </div>
        </div>
        <div className={s.body}>
          {isPassword
            ? r.oneOff
              ? t("terminal.password.oneOffBody")
              : t("terminal.password.body", { target: r.target })
            : t("terminal.passphrase.body", { name: r.hostName })}
        </div>
        {r.wrong && <div className={s.wrong}>{t("terminal.passphrase.wrong")}</div>}
        <div className={s.fields}>
          <TextField
            secret
            large
            mono
            autoFocus
            aria-label={isPassword ? t("terminal.password.label") : t("terminal.passphrase.label")}
            placeholder={isPassword ? t("terminal.password.label") : t("terminal.passphrase.label")}
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
        </div>
        <div className={s.actions}>
          <Button onClick={cancel}>{t("btn.cancel")}</Button>
          <Button variant="primary" type="submit" disabled={!value}>
            {t("terminal.connect")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}

/** Quick connect without a user name (`ssh host`, HOST-12): who to log in as. */
function UsernameDialog({ item }: { item: PromptItem & { kind: "username" } }) {
  const t = useT();
  const [value, setValue] = useState("");
  const name = value.trim();
  const cancel = useCallback(() => answerUsername(item, null), [item]);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (name && !/\s/.test(name)) answerUsername(item, name);
  };
  return (
    <Modal center onClose={cancel} closeOnBackdrop={false}>
      <form role="dialog" aria-modal aria-labelledby="user-title" className={s.card} onSubmit={submit}>
        <div className={s.head}>
          <Icon name="user" size={20} color="var(--accent)" />
          <div id="user-title" className={s.title}>
            {t("terminal.user.title", { target: item.target })}
          </div>
        </div>
        <div className={s.body}>{t("terminal.user.body")}</div>
        <div className={s.fields}>
          <TextField
            large
            mono
            autoFocus
            spellCheck={false}
            autoCapitalize="off"
            aria-label={t("terminal.user.label")}
            placeholder={t("terminal.user.label")}
            value={value}
            onChange={(e) => setValue(e.target.value)}
          />
        </div>
        <div className={s.actions}>
          <Button onClick={cancel}>{t("btn.cancel")}</Button>
          <Button variant="primary" type="submit" disabled={!name || /\s/.test(name)}>
            {t("terminal.connect")}
          </Button>
        </div>
      </form>
    </Modal>
  );
}
