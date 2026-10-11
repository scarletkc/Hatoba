import { useId, useRef, useState } from "react";
import { Button, Segmented, TextArea, TextField } from "@/components/controls";
import { Dialog, FooterSpacer, PopupSelect, Sheet, SheetHeader, toast } from "@/components/overlay";
import { errorMessage } from "@/app/errors";
import { formatDate, useT } from "@/i18n";
import { api, isTauri, toAppError } from "@/ipc/api";
import type { HostView, KeyView } from "@/ipc/types";
import { Field } from "./Field";
import s from "./KeyDialogs.module.css";

// ───────────────────────── Import (KEY-01) ─────────────────────────

/** A file the user picked: a native path in the app, or file contents in a plain browser. */
interface PickedFile {
  path: string | null;
  text: string | null;
  label: string;
}

async function pickKeyFile(title: string): Promise<PickedFile | null> {
  if (isTauri()) {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const path = await open({ multiple: false, directory: false, title });
    return typeof path === "string" ? { path, text: null, label: path } : null;
  }
  // Browser fallback (dev server): read the file in the page.
  const file = await new Promise<File | null>((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
  return file ? { path: null, text: await file.text(), label: file.name } : null;
}

const baseName = (path: string) => (path.split(/[\\/]/).pop() ?? path).replace(/\.(ppk|pem|key)$/i, "");

type ImportField = "name" | "source" | "passphrase";

export function ImportKeyDialog({ onClose, onDone }: { onClose: () => void; onDone: (key: KeyView) => void }) {
  const t = useT();
  const nameId = useId();
  const passId = useId();
  const passRef = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");
  const [source, setSource] = useState<"file" | "paste">("file");
  const [file, setFile] = useState<PickedFile | null>(null);
  const [text, setText] = useState("");
  const [passphrase, setPassphrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<{ field: ImportField; message: string } | null>(null);

  const choose = async () => {
    try {
      const picked = await pickKeyFile(t("keys.import.title"));
      if (!picked) return;
      setFile(picked);
      setError(null);
      if (!name.trim()) setName(baseName(picked.label));
    } catch (e) {
      setError({ field: "source", message: errorMessage(t, e) });
    }
  };

  const submit = async () => {
    if (!name.trim()) return setError({ field: "name", message: t("keys.field.nameRequired") });
    const hasSource = source === "file" ? !!file : text.trim().length > 0;
    if (!hasSource) return setError({ field: "source", message: t("keys.import.sourceRequired") });
    setBusy(true);
    setError(null);
    try {
      const key = await api.key_import({
        name: name.trim(),
        path: source === "file" ? file?.path ?? null : null,
        private_key: source === "paste" ? text : file?.path ? null : (file?.text ?? null),
        passphrase: passphrase || null,
      });
      onDone(key);
    } catch (e) {
      const err = toAppError(e);
      const onPassphrase =
        err.code === "key_parse" && (err.key_kind === "passphrase_required" || err.key_kind === "wrong_passphrase");
      setError({ field: onPassphrase ? "passphrase" : "source", message: errorMessage(t, e) });
      if (onPassphrase) passRef.current?.focus();
      setBusy(false);
    }
  };

  const formId = useId();
  return (
    <Sheet
      width={480}
      onClose={busy ? undefined : onClose}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy}>
            {t("keys.import.action")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("keys.import.title")} subtitle={t("keys.import.body")} />
      <form
        id={formId}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field label={t("keys.field.name")} htmlFor={nameId} error={error?.field === "name" ? error.message : undefined}>
          <TextField
            id={nameId}
            value={name}
            invalid={error?.field === "name"}
            placeholder={t("keys.field.namePlaceholder")}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
        </Field>
        <Field label={t("keys.import.source")} error={error?.field === "source" ? error.message : undefined}>
          <Segmented
            ariaLabel={t("keys.import.source")}
            value={source}
            onChange={setSource}
            options={[
              { value: "file", label: t("keys.import.file"), icon: "file" },
              { value: "paste", label: t("keys.import.paste"), icon: "clipboard-text" },
            ]}
          />
          {source === "file" ? (
            <div className={s.filePick}>
              <Button size="sm" icon="folder-open" onClick={() => void choose()}>
                {t("keys.import.chooseFile")}
              </Button>
              <span className={file ? s.fileName : s.fileNone} title={file?.label}>
                {file ? file.label : t("keys.import.noFile")}
              </span>
            </div>
          ) : (
            <TextArea
              className={s.paste}
              value={text}
              rows={5}
              placeholder={t("keys.import.pastePlaceholder")}
              aria-label={t("keys.import.paste")}
              onChange={(e) => setText(e.target.value)}
            />
          )}
        </Field>
        <Field
          label={t("keys.field.passphraseOpt")}
          htmlFor={passId}
          error={error?.field === "passphrase" ? error.message : undefined}
        >
          <TextField
            id={passId}
            ref={passRef}
            secret
            value={passphrase}
            invalid={error?.field === "passphrase"}
            onChange={(e) => setPassphrase(e.target.value)}
          />
        </Field>
      </form>
    </Sheet>
  );
}

// ───────────────────────── Generate (KEY-02) ─────────────────────────

export function GenerateKeyDialog({ onClose, onDone }: { onClose: () => void; onDone: (key: KeyView) => void }) {
  const t = useT();
  const nameId = useId();
  const commentId = useId();
  const passId = useId();
  const confirmId = useId();
  const formId = useId();
  const [name, setName] = useState(() => `hatoba-${formatDate(Date.now())}`);
  const [algorithm, setAlgorithm] = useState<"ed25519" | "rsa">("ed25519");
  const [comment, setComment] = useState("user@hatoba");
  const [passphrase, setPassphrase] = useState("");
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nameError, setNameError] = useState<string | null>(null);

  const mismatch = confirmation.length > 0 && confirmation !== passphrase;
  const submit = async () => {
    if (!name.trim()) return setNameError(t("keys.field.nameRequired"));
    if (passphrase !== confirmation) return setError(t("keys.generate.mismatch"));
    setBusy(true);
    setError(null);
    try {
      onDone(
        await api.key_generate({
          name: name.trim(),
          algorithm,
          comment: comment.trim(),
          passphrase: passphrase || null,
        }),
      );
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  };

  return (
    <Sheet
      width={480}
      onClose={busy ? undefined : onClose}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy}>
            {t("keys.generate.action")}
          </Button>
        </>
      }
    >
      <SheetHeader title={t("keys.generate.title")} />
      <form
        id={formId}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field label={t("keys.field.name")} htmlFor={nameId} error={nameError}>
          <TextField
            id={nameId}
            value={name}
            invalid={!!nameError}
            disabled={busy}
            onChange={(e) => {
              setName(e.target.value);
              setNameError(null);
            }}
            autoFocus
          />
        </Field>
        <Field
          label={t("keys.generate.type")}
          hint={algorithm === "ed25519" ? t("keys.generate.hintEd25519") : t("keys.generate.hintRsa")}
        >
          <Segmented
            ariaLabel={t("keys.generate.type")}
            value={algorithm}
            onChange={setAlgorithm}
            options={[
              { value: "ed25519", label: t("keys.generate.typeEd25519") },
              { value: "rsa", label: t("keys.generate.typeRsa") },
            ]}
          />
        </Field>
        <Field label={t("keys.generate.comment")} htmlFor={commentId}>
          <TextField id={commentId} mono value={comment} disabled={busy} onChange={(e) => setComment(e.target.value)} />
        </Field>
        <Field label={t("keys.field.passphraseOpt")} htmlFor={passId}>
          <TextField id={passId} secret value={passphrase} disabled={busy} onChange={(e) => setPassphrase(e.target.value)} />
        </Field>
        <Field
          label={t("keys.generate.confirmPassphrase")}
          htmlFor={confirmId}
          error={mismatch ? t("keys.generate.mismatch") : undefined}
        >
          <TextField
            id={confirmId}
            secret
            value={confirmation}
            invalid={mismatch}
            disabled={busy}
            onChange={(e) => setConfirmation(e.target.value)}
          />
        </Field>
        {error && (
          <div className={s.formError} role="alert">
            {error}
          </div>
        )}
      </form>
    </Sheet>
  );
}

// ───────────────────────── Rename ─────────────────────────

export function RenameKeyDialog({ keyView, onClose, onDone }: { keyView: KeyView; onClose: () => void; onDone: () => void }) {
  const t = useT();
  const nameId = useId();
  const formId = useId();
  const [name, setName] = useState(keyView.name);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    const next = name.trim();
    if (!next) return setError(t("keys.field.nameRequired"));
    if (next === keyView.name) return onClose();
    setBusy(true);
    try {
      await api.key_rename(keyView.id, next);
      onDone();
    } catch (e) {
      setError(errorMessage(t, e));
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t("keys.rename.title")}
      icon="pencil-simple"
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={formId} busy={busy}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <form
        id={formId}
        className={s.dialogForm}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Field label={t("keys.field.name")} htmlFor={nameId} error={error}>
          <TextField id={nameId} value={name} invalid={!!error} onChange={(e) => setName(e.target.value)} autoFocus />
        </Field>
      </form>
    </Dialog>
  );
}

// ───────────────────────── Deploy (KEY-06) ─────────────────────────

export function DeployKeyDialog({ keyView, hosts, onClose }: { keyView: KeyView; hosts: HostView[]; onClose: () => void }) {
  const t = useT();
  const [hostId, setHostId] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const sorted = [...hosts].sort((a, b) => a.name.localeCompare(b.name));
  const host = hosts.find((h) => h.id === hostId);

  const deploy = async () => {
    if (!host) return;
    setBusy(true);
    setError(null);
    try {
      await api.key_deploy(keyView.id, host.id);
      toast(t("keys.deploy.done", { host: host.name }), "success");
      onClose();
    } catch (e) {
      setError(errorMessage(t, e, { host: host.address, port: host.port }));
      setBusy(false);
    }
  };

  return (
    <Dialog
      title={t("keys.deploy.title")}
      icon="upload-simple"
      body={t("keys.deploy.body", { name: keyView.name })}
      onClose={busy ? undefined : onClose}
      actions={
        <>
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" onClick={() => void deploy()} busy={busy} disabled={!host}>
            {t("keys.deploy.action")}
          </Button>
        </>
      }
    >
      <div className={s.dialogForm}>
        {sorted.length === 0 ? (
          <div className={s.formHint}>{t("keys.deploy.noHosts")}</div>
        ) : (
          <Field label={t("keys.deploy.host")} error={error}>
            <PopupSelect
              ariaLabel={t("keys.deploy.host")}
              icon="hard-drives"
              value={hostId}
              placeholder={t("keys.deploy.choose")}
              disabled={busy}
              onChange={(id) => {
                setHostId(id);
                setError(null);
              }}
              options={sorted.map((h) => ({ value: h.id, label: h.name, hint: `${h.username}@${h.address}` }))}
            />
          </Field>
        )}
      </div>
    </Dialog>
  );
}
