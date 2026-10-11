import { useEffect, useId, useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Checkbox, Icon, TextField } from "@/components/controls";
import { FormRow, Group } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader } from "@/components/overlay";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { ForwardView } from "@/ipc/types";
import s from "./ForwardDialog.module.css";

type FieldKey = "bind_address" | "bind_port" | "dest_host" | "dest_port";
type Errors = Partial<Record<FieldKey, string>>;

const FIELDS: readonly string[] = ["bind_address", "bind_port", "dest_host", "dest_port"];
const LABEL_COLUMN = "120px minmax(0,1fr)";

/** A whole number in [min, 65535]. */
const validPort = (text: string, min: number) => {
  const v = text.trim();
  return /^\d{1,5}$/.test(v) && Number(v) >= min && Number(v) <= 65535;
};

/** Addresses that make the forwarded port reachable from other machines. */
const exposesPort = (address: string) => {
  const a = address.trim().toLowerCase();
  return a !== "" && a !== "localhost" && a !== "::1" && !a.startsWith("127.");
};

/** Add / edit one local forward (FWD-01, FWD-02). Saved straight to the backend, not with the host form. */
export function ForwardDialog({
  hostId,
  forward,
  onClose,
  onSaved,
}: {
  hostId: string;
  forward: ForwardView | null;
  onClose: () => void;
  onSaved: (saved: ForwardView) => void;
}) {
  const t = useT();
  const ids = { form: useId(), bind: useId(), bindPort: useId(), dest: useId(), destPort: useId() };
  const bindPortRef = useRef<HTMLInputElement>(null);
  const [bindAddress, setBindAddress] = useState(forward?.bind_address ?? "127.0.0.1");
  const [bindPort, setBindPort] = useState(forward ? String(forward.bind_port) : "");
  const [destHost, setDestHost] = useState(forward?.dest_host ?? "127.0.0.1");
  const [destPort, setDestPort] = useState(forward ? String(forward.dest_port) : "");
  const [autoStart, setAutoStart] = useState(forward?.auto_start ?? false);
  const [errors, setErrors] = useState<Errors>({});
  const [general, setGeneral] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The address is almost always the default, so a new forward starts on the port.
  useEffect(() => {
    if (!forward) bindPortRef.current?.focus();
  }, [forward]);

  const clear = (field: FieldKey) => {
    setErrors((e) => (e[field] ? { ...e, [field]: undefined } : e));
    setGeneral(null);
  };

  const validate = (): Errors => {
    const e: Errors = {};
    if (!bindAddress.trim()) e.bind_address = t("hosts.fwd.err.bindRequired");
    if (!validPort(bindPort, 0)) e.bind_port = t("hosts.fwd.err.localPort");
    if (!destHost.trim()) e.dest_host = t("hosts.fwd.err.destRequired");
    if (!validPort(destPort, 1)) e.dest_port = t("hosts.fwd.err.destPort");
    return e;
  };

  const fieldIds: Record<FieldKey, string> = {
    bind_address: ids.bind,
    bind_port: ids.bindPort,
    dest_host: ids.dest,
    dest_port: ids.destPort,
  };
  const focusFirst = (e: Errors) => {
    const first = (Object.keys(fieldIds) as FieldKey[]).find((k) => e[k]);
    if (first) document.getElementById(fieldIds[first])?.focus();
  };

  const submit = async () => {
    if (busy) return;
    const e = validate();
    if (Object.values(e).some(Boolean)) {
      setErrors(e);
      focusFirst(e);
      return;
    }
    setBusy(true);
    setGeneral(null);
    try {
      const saved = await api.forward_save({
        id: forward?.id ?? null,
        host_id: hostId,
        bind_address: bindAddress.trim(),
        bind_port: Number(bindPort),
        dest_host: destHost.trim(),
        dest_port: Number(destPort),
        auto_start: autoStart,
      });
      onSaved(saved);
    } catch (err) {
      const ae = toAppError(err);
      const message = errorMessage(t, err);
      if (ae.code === "invalid_input" && ae.field && FIELDS.includes(ae.field)) {
        const next = { [ae.field]: message } as Errors;
        setErrors(next);
        focusFirst(next);
      } else {
        setGeneral(message);
      }
      setBusy(false);
    }
  };

  const exposed = exposesPort(bindAddress);
  const localError = [errors.bind_address, errors.bind_port].filter(Boolean).join(" ");
  const destError = [errors.dest_host, errors.dest_port].filter(Boolean).join(" ");

  return (
    <Sheet
      width={520}
      onClose={busy ? undefined : onClose}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={ids.form} busy={busy}>
            {forward ? t("btn.save") : t("hosts.fwd.dialog.add")}
          </Button>
        </>
      }
    >
      <SheetHeader
        title={forward ? t("hosts.fwd.dialog.editTitle") : t("hosts.fwd.dialog.addTitle")}
        subtitle={t("hosts.fwd.dialog.subtitle")}
      />
      {/* Keys stay inside the dialog: the host editor behind it saves on Enter (React events bubble through portals). */}
      <form
        id={ids.form}
        className={s.form}
        noValidate
        onKeyDown={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        {general && (
          <div className={s.general} role="alert">
            <Icon name="warning-circle" size={15} />
            {general}
          </div>
        )}
        <Group>
          <FormRow label={t("hosts.fwd.f.local")} htmlFor={ids.bind} columns={LABEL_COLUMN} top>
            <div className={s.stack}>
              <div className={s.addrRow}>
                <TextField
                  id={ids.bind}
                  mono
                  value={bindAddress}
                  invalid={!!errors.bind_address}
                  onChange={(e) => {
                    setBindAddress(e.target.value);
                    clear("bind_address");
                  }}
                />
                <label className={s.portLabel} htmlFor={ids.bindPort}>
                  {t("hosts.f.port")}
                </label>
                <TextField
                  id={ids.bindPort}
                  ref={bindPortRef}
                  mono
                  inputMode="numeric"
                  placeholder={t("hosts.fwd.f.portHint")}
                  value={bindPort}
                  invalid={!!errors.bind_port}
                  onChange={(e) => {
                    setBindPort(e.target.value);
                    clear("bind_port");
                  }}
                />
              </div>
              {localError ? (
                <div className={s.error} role="alert">
                  {localError}
                </div>
              ) : (
                exposed && (
                  <div className={s.warn}>
                    <Icon name="warning" size={13} />
                    {t("hosts.fwd.f.exposedWarn")}
                  </div>
                )
              )}
            </div>
          </FormRow>
          <FormRow label={t("hosts.fwd.f.dest")} htmlFor={ids.dest} columns={LABEL_COLUMN} top>
            <div className={s.stack}>
              <div className={s.addrRow}>
                <TextField
                  id={ids.dest}
                  mono
                  value={destHost}
                  invalid={!!errors.dest_host}
                  onChange={(e) => {
                    setDestHost(e.target.value);
                    clear("dest_host");
                  }}
                />
                <label className={s.portLabel} htmlFor={ids.destPort}>
                  {t("hosts.f.port")}
                </label>
                <TextField
                  id={ids.destPort}
                  mono
                  inputMode="numeric"
                  value={destPort}
                  invalid={!!errors.dest_port}
                  onChange={(e) => {
                    setDestPort(e.target.value);
                    clear("dest_port");
                  }}
                />
              </div>
              {destError ? (
                <div className={s.error} role="alert">
                  {destError}
                </div>
              ) : (
                <div className={s.hint}>{t("hosts.fwd.f.destHint")}</div>
              )}
            </div>
          </FormRow>
          <FormRow label="" columns={LABEL_COLUMN}>
            <Checkbox checked={autoStart} onChange={setAutoStart}>
              {t("hosts.fwd.f.autoStart")}
            </Checkbox>
          </FormRow>
        </Group>
      </form>
    </Sheet>
  );
}
