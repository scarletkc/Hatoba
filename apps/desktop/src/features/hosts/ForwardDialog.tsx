import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useVaultData } from "@/app/data";
import { errorMessage } from "@/app/errors";
import { Button, Checkbox, Icon, Segmented, TextField } from "@/components/controls";
import { FormRow, Group } from "@/components/layout";
import { FooterSpacer, PopupSelect, Sheet, SheetHeader, type SelectOption } from "@/components/overlay";
import { ErrorText, HintText, WarningText } from "@/components/patterns";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { ForwardKind, ForwardView } from "@/ipc/types";
import s from "./ForwardDialog.module.css";

type FieldKey = "name" | "host_id" | "bind_address" | "bind_port" | "dest_host" | "dest_port";
type Errors = Partial<Record<FieldKey, string>>;

const FIELDS: readonly string[] = ["name", "host_id", "bind_address", "bind_port", "dest_host", "dest_port"];
const LABEL_COLUMN = "120px minmax(0,1fr)";
const KINDS: readonly ForwardKind[] = ["local", "remote", "dynamic"];

const KIND_TEXT: Record<ForwardKind, { label: MessageKey; subtitle: MessageKey }> = {
  local: { label: "hosts.fwd.kind.local", subtitle: "hosts.fwd.dialog.subtitle" },
  remote: { label: "hosts.fwd.kind.remote", subtitle: "hosts.fwd.dialog.subtitleRemote" },
  dynamic: { label: "hosts.fwd.kind.dynamic", subtitle: "hosts.fwd.dialog.subtitleDynamic" },
};

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

/** Where a new forward listens: a remote one where `ssh -R` does. */
const defaultBind = (kind: ForwardKind) => (kind === "remote" ? "localhost" : "127.0.0.1");

/**
 * Add / edit one forward: its optional name (FWD-06), local (FWD-01), remote (FWD-03) or dynamic SOCKS (FWD-04), and
 * whether it starts with the connection (FWD-02), saved straight to the backend. A new one chooses its host, at first
 * `defaultHostId`; an edited one stays on `hostId`.
 */
export function ForwardDialog({
  hostId,
  defaultHostId = null,
  forward,
  onClose,
  onSaved,
}: {
  hostId: string | null;
  defaultHostId?: string | null;
  forward: ForwardView | null;
  onClose: () => void;
  onSaved: (saved: ForwardView) => void;
}) {
  const t = useT();
  const hosts = useVaultData((st) => st.hosts);
  const ids = { form: useId(), name: useId(), bind: useId(), bindPort: useId(), dest: useId(), destPort: useId() };
  const bindPortRef = useRef<HTMLInputElement>(null);
  const [chosenHost, setChosenHost] = useState<string | null>(
    hostId ?? (hosts.some((h) => h.id === defaultHostId) ? defaultHostId : null),
  );
  const hostOptions: SelectOption<string | null>[] = useMemo(
    () =>
      [...hosts]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((h) => ({ value: h.id, label: h.name, hint: `${h.username ? `${h.username}@` : ""}${h.address}:${h.port}` })),
    [hosts],
  );
  const [name, setName] = useState(forward?.name ?? "");
  const [kind, setKind] = useState<ForwardKind>(forward?.kind ?? "local");
  const [bindAddress, setBindAddress] = useState(forward?.bind_address ?? defaultBind("local"));
  const [bindPort, setBindPort] = useState(forward ? String(forward.bind_port) : "");
  // A dynamic forward has no destination: one changed to another kind starts from the usual one.
  const [destHost, setDestHost] = useState(forward?.dest_host || "127.0.0.1");
  const [destPort, setDestPort] = useState(forward?.dest_port ? String(forward.dest_port) : "");
  const [autoStart, setAutoStart] = useState(forward?.auto_start ?? false);
  const [errors, setErrors] = useState<Errors>({});
  const [general, setGeneral] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // The address is almost always the default, so a new forward starts on the port.
  useEffect(() => {
    if (!forward) bindPortRef.current?.focus();
  }, [forward]);

  const remote = kind === "remote";
  const hasDest = kind !== "dynamic";

  const clear = (field: FieldKey) => {
    setErrors((e) => (e[field] ? { ...e, [field]: undefined } : e));
    setGeneral(null);
  };

  const changeKind = (next: ForwardKind) => {
    // An address left at one kind's default moves to the next one's.
    if (bindAddress.trim() === defaultBind(kind)) setBindAddress(defaultBind(next));
    setKind(next);
    setErrors((e) => ({ name: e.name, host_id: e.host_id }));
    setGeneral(null);
  };

  const validate = (): Errors => {
    const e: Errors = {};
    if (!chosenHost) e.host_id = t("hosts.fwd.err.hostRequired");
    if (!bindAddress.trim()) e.bind_address = t(remote ? "hosts.fwd.err.serverBindRequired" : "hosts.fwd.err.bindRequired");
    if (!validPort(bindPort, 0)) e.bind_port = t(remote ? "hosts.fwd.err.serverPort" : "hosts.fwd.err.localPort");
    if (hasDest && !destHost.trim()) e.dest_host = t("hosts.fwd.err.destRequired");
    if (hasDest && !validPort(destPort, 1)) e.dest_port = t("hosts.fwd.err.destPort");
    return e;
  };

  const fieldIds: Partial<Record<FieldKey, string>> = {
    name: ids.name,
    bind_address: ids.bind,
    bind_port: ids.bindPort,
    dest_host: ids.dest,
    dest_port: ids.destPort,
  };
  const focusFirst = (e: Errors) => {
    const first = (Object.keys(fieldIds) as FieldKey[]).find((k) => e[k]);
    if (first) document.getElementById(fieldIds[first]!)?.focus();
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
        host_id: chosenHost!,
        name: name.trim(),
        kind,
        bind_address: bindAddress.trim(),
        bind_port: Number(bindPort),
        dest_host: hasDest ? destHost.trim() : "",
        dest_port: hasDest ? Number(destPort) : 0,
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
  const bindError = [errors.bind_address, errors.bind_port].filter(Boolean).join(" ");
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
        subtitle={t(KIND_TEXT[kind].subtitle)}
      />
      {/* Keys stay inside the dialog: React events bubble through portals to the page behind it. */}
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
            <Icon name="warning-circle" size={16} />
            {general}
          </div>
        )}
        <Group>
          <FormRow label={t("hosts.fwd.f.name")} htmlFor={ids.name} columns={LABEL_COLUMN} error={errors.name}>
            <TextField
              id={ids.name}
              value={name}
              invalid={!!errors.name}
              placeholder={t("hosts.fwd.f.namePlaceholder")}
              onChange={(e) => {
                setName(e.target.value);
                clear("name");
              }}
            />
          </FormRow>
          {hostId === null && (
            <FormRow label={t("hosts.fwd.f.host")} columns={LABEL_COLUMN} error={errors.host_id}>
              <PopupSelect<string | null>
                ariaLabel={t("hosts.fwd.f.host")}
                icon="hard-drives"
                value={chosenHost}
                options={hostOptions}
                placeholder={t("hosts.fwd.f.hostPick")}
                minWidth={320}
                onChange={(id) => {
                  setChosenHost(id);
                  clear("host_id");
                }}
              />
            </FormRow>
          )}
          <FormRow label={t("hosts.fwd.f.kind")} columns={LABEL_COLUMN}>
            <Segmented<ForwardKind>
              ariaLabel={t("hosts.fwd.f.kind")}
              value={kind}
              options={KINDS.map((k) => ({ value: k, label: t(KIND_TEXT[k].label) }))}
              onChange={changeKind}
            />
          </FormRow>
          <FormRow label={t(remote ? "hosts.fwd.f.server" : "hosts.fwd.f.local")} htmlFor={ids.bind} columns={LABEL_COLUMN} top>
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
              {bindError ? (
                <ErrorText>{bindError}</ErrorText>
              ) : exposed ? (
                <WarningText>{t(remote ? "hosts.fwd.f.gatewayPorts" : "hosts.fwd.f.exposedWarn")}</WarningText>
              ) : (
                kind === "dynamic" && <HintText>{t("hosts.fwd.f.socksHint")}</HintText>
              )}
            </div>
          </FormRow>
          {hasDest && (
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
                  <ErrorText>{destError}</ErrorText>
                ) : (
                  <HintText>{t(remote ? "hosts.fwd.f.destHintRemote" : "hosts.fwd.f.destHint")}</HintText>
                )}
              </div>
            </FormRow>
          )}
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
