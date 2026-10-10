import { useEffect, useId, useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, IconButton, LinkButton, Segmented, Switch, TextField } from "@/components/controls";
import { FormRow, Group, Section } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader, confirm } from "@/components/overlay";
import { useT } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { McpServerView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { SavedKeyField } from "./aiShared";
import {
  argRows,
  commandPreview,
  hasMcpProblems,
  initialMcpForm,
  looksLikeCommandLine,
  mcpFormToInput,
  newSecretRow,
  newUid,
  splitCommandLine,
  validateMcpForm,
  type McpForm,
  type SecretKind,
  type SecretRow,
  type SecretRowProblem,
  type TransportKind,
} from "./mcpLogic";
import s from "./McpSection.module.css";

type Rejected = Partial<Record<"name" | "command" | "url" | "general", string>>;

/**
 * Add or edit one MCP server (AI-29). Saved straight to the backend. Before a `stdio` server is saved, a confirmation
 * shows the full command line, because the server is a program that runs on this device with the user's privileges.
 */
export function McpServerDialog({
  server,
  otherNames,
  onClose,
  onSaved,
}: {
  /** null adds a server. */
  server: McpServerView | null;
  /** The names of the other saved servers. */
  otherNames: string[];
  onClose: () => void;
  onSaved: (saved: McpServerView) => void;
}) {
  const t = useT();
  const ids = { form: useId(), name: useId(), command: useId(), url: useId() };
  const [form, setForm] = useState<McpForm>(() => initialMcpForm(server));
  const [submitted, setSubmitted] = useState(false);
  const [rejected, setRejected] = useState<Rejected>({});
  const [splitFailed, setSplitFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const [asking, setAsking] = useState(false);
  const [focusUid, setFocusUid] = useState<number | null>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const problems = validateMcpForm(form, otherNames);
  const busy = saving || asking;

  const patch = (p: Partial<McpForm>) => {
    setForm((f) => ({ ...f, ...p }));
    setRejected({});
  };

  // An empty field is only a problem once the user tried to save; everything else shows as it is typed.
  const show = <T extends string | null>(problem: T): T | null => (problem !== "empty" || submitted ? problem : null);
  const nameProblem = show(problems.name);
  const commandProblem = show(problems.command);
  const urlProblem = show(problems.url);

  const nameError = rejected.name ?? (nameProblem ? t(nameProblem === "empty" ? "aiSettings.mcp.err.nameRequired" : "aiSettings.mcp.err.nameTaken") : null);
  const commandError = rejected.command ?? (commandProblem ? t("aiSettings.mcp.err.commandRequired") : null);
  const urlError =
    rejected.url ??
    (urlProblem ? t(urlProblem === "empty" ? "aiSettings.mcp.err.urlRequired" : "aiSettings.mcp.err.urlInvalid") : null);

  const splitCommand = () => {
    const words = splitCommandLine(form.command);
    setSplitFailed(words === null);
    if (words === null || words.length === 0) return;
    patch({ command: words[0], args: argRows(words.slice(1)) });
  };

  const save = async () => {
    if (busy) return;
    setSubmitted(true);
    if (hasMcpProblems(problems)) {
      if (problems.name) document.getElementById(ids.name)?.focus();
      else if (problems.command) document.getElementById(ids.command)?.focus();
      else if (problems.url) document.getElementById(ids.url)?.focus();
      return;
    }
    if (form.kind === "stdio") {
      const { line, envNames } = commandPreview(form);
      setAsking(true);
      const ok = await confirm({
        title: t("aiSettings.mcp.confirm.title"),
        body: (
          <>
            <p className={s.confirmText}>{t("aiSettings.mcp.confirm.body")}</p>
            <pre className={cx(s.commandLine, "selectable")}>{line}</pre>
            <p className={s.confirmText}>
              {envNames.length ? t("aiSettings.mcp.confirm.env", { names: envNames.join(", ") }) : t("aiSettings.mcp.confirm.noEnv")}
            </p>
          </>
        ),
        confirmLabel: t("aiSettings.mcp.confirm.submit"),
        icon: "terminal-window",
        iconColor: "var(--orange)",
      });
      if (!live.current) return;
      setAsking(false);
      if (!ok) return;
    }
    setSaving(true);
    try {
      const saved = await api.mcp_server_save(mcpFormToInput(server?.id ?? null, form));
      onSaved(saved);
      onClose();
    } catch (e) {
      const err = toAppError(e);
      if (err.code === "invalid_input" && err.field === "name") setRejected({ name: t("aiSettings.mcp.err.nameRequired") });
      else if (err.code === "invalid_input" && err.field === "command") setRejected({ command: t("aiSettings.mcp.err.commandRequired") });
      else if (err.code === "invalid_input" && err.field === "url") setRejected({ url: t("aiSettings.mcp.err.urlRejected") });
      else setRejected({ general: [errorMessage(t, e), err.code === "invalid_input" ? err.detail : ""].filter(Boolean).join("\n") });
      setSaving(false);
    }
  };

  const setArg = (uid: number, value: string) => patch({ args: form.args.map((a) => (a.uid === uid ? { ...a, value } : a)) });
  const addArg = (after?: number) => {
    const row = { uid: newUid(), value: "" };
    const at = after === undefined ? form.args.length : form.args.findIndex((a) => a.uid === after) + 1;
    patch({ args: [...form.args.slice(0, at), row, ...form.args.slice(at)] });
    setFocusUid(row.uid);
  };

  const secrets = form.kind === "stdio" ? form.env : form.headers;
  const setSecrets = (rows: SecretRow[]) => patch(form.kind === "stdio" ? { env: rows } : { headers: rows });

  return (
    <Sheet
      width={680}
      onClose={busy ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          <FooterSpacer />
          <Button onClick={onClose} disabled={busy}>
            {t("btn.cancel")}
          </Button>
          <Button variant="primary" type="submit" form={ids.form} busy={saving} disabled={asking}>
            {t("btn.save")}
          </Button>
        </>
      }
    >
      <SheetHeader title={server ? t("aiSettings.mcp.dlg.editTitle") : t("aiSettings.mcp.dlg.addTitle")} subtitle={t("aiSettings.mcp.dlg.subtitle")} />
      <form
        id={ids.form}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void save();
        }}
      >
        <Group>
          <FormRow label={t("aiSettings.mcp.f.name")} htmlFor={ids.name} error={nameError}>
            <TextField
              id={ids.name}
              autoFocus
              value={form.name}
              invalid={!!nameError}
              placeholder={t("aiSettings.mcp.f.namePlaceholder")}
              onChange={(e) => patch({ name: e.target.value })}
            />
          </FormRow>
          <FormRow label={t("aiSettings.mcp.f.transport")} top>
            <div className={s.stack}>
              <Segmented<TransportKind>
                ariaLabel={t("aiSettings.mcp.f.transport")}
                value={form.kind}
                options={[
                  { value: "stdio", label: t("aiSettings.mcp.transport.stdio") },
                  { value: "http", label: t("aiSettings.mcp.transport.http") },
                ]}
                onChange={(kind) => patch({ kind })}
              />
              <span className={s.hint}>{t(form.kind === "stdio" ? "aiSettings.mcp.transport.stdioHint" : "aiSettings.mcp.transport.httpHint")}</span>
            </div>
          </FormRow>
          {form.kind === "stdio" ? (
            <>
              <FormRow label={t("aiSettings.mcp.f.command")} htmlFor={ids.command} top error={commandError}>
                <div className={s.stack}>
                  <TextField
                    id={ids.command}
                    mono
                    value={form.command}
                    invalid={!!commandError}
                    placeholder={t("aiSettings.mcp.f.commandPlaceholder")}
                    onChange={(e) => {
                      setSplitFailed(false);
                      patch({ command: e.target.value });
                    }}
                  />
                  {looksLikeCommandLine(form.command) && form.args.length === 0 ? (
                    <span className={cx(s.hint, s.hintWarn)}>
                      {splitFailed ? t("aiSettings.mcp.f.splitFailed") : t("aiSettings.mcp.f.commandLine")}{" "}
                      <LinkButton onClick={splitCommand}>{t("aiSettings.mcp.f.split")}</LinkButton>
                    </span>
                  ) : (
                    <span className={s.hint}>{t("aiSettings.mcp.f.commandHint")}</span>
                  )}
                </div>
              </FormRow>
              <FormRow label={t("aiSettings.mcp.f.args")} top>
                <div className={s.stack}>
                  {form.args.map((arg, i) => (
                    <div className={s.argRow} key={arg.uid}>
                      <TextField
                        mono
                        autoFocus={focusUid === arg.uid}
                        aria-label={`${t("aiSettings.mcp.f.argPlaceholder")} ${i + 1}`}
                        value={arg.value}
                        placeholder={t("aiSettings.mcp.f.argPlaceholder")}
                        onChange={(e) => setArg(arg.uid, e.target.value)}
                        onKeyDown={(e) => {
                          // Enter adds the next argument; it must not save the whole form.
                          if (e.key !== "Enter" || isImeEvent(e)) return;
                          e.preventDefault();
                          addArg(arg.uid);
                        }}
                      />
                      <IconButton
                        icon="x"
                        size={13}
                        label={t("aiSettings.mcp.f.argRemove", { n: i + 1 })}
                        onClick={() => patch({ args: form.args.filter((a) => a.uid !== arg.uid) })}
                      />
                    </div>
                  ))}
                  <div>
                    <Button size="sm" icon="plus" onClick={() => addArg()}>
                      {t("aiSettings.mcp.f.argsAdd")}
                    </Button>
                  </div>
                </div>
              </FormRow>
            </>
          ) : (
            <FormRow label={t("aiSettings.mcp.f.url")} htmlFor={ids.url} top error={urlError}>
              <div className={s.stack}>
                <TextField
                  id={ids.url}
                  mono
                  value={form.url}
                  invalid={!!urlError}
                  placeholder={t("aiSettings.mcp.f.urlPlaceholder")}
                  onChange={(e) => patch({ url: e.target.value })}
                />
                <span className={s.hint}>{t("aiSettings.mcp.f.urlHint")}</span>
              </div>
            </FormRow>
          )}
        </Group>

        <Section title={form.kind === "stdio" ? t("aiSettings.mcp.f.env") : t("aiSettings.mcp.f.headers")}>
          <div className={s.hint}>{form.kind === "stdio" ? t("aiSettings.mcp.f.envHint") : t("aiSettings.mcp.f.headersHint")}</div>
          <SecretRows
            // The two transports keep their own rows; switching must not carry one's focus or values into the other.
            key={form.kind}
            kind={form.kind === "stdio" ? "env" : "header"}
            rows={secrets}
            problems={problems.secrets}
            onChange={setSecrets}
          />
        </Section>

        <Group>
          <FormRow label={t("aiSettings.mcp.f.alwaysAsk")} top>
            <div className={s.alwaysAsk}>
              <Switch checked={form.alwaysAsk} label={t("aiSettings.mcp.f.alwaysAsk")} onChange={(alwaysAsk) => patch({ alwaysAsk })} />
              <span className={s.hint}>{t("aiSettings.mcp.f.alwaysAskHint")}</span>
            </div>
          </FormRow>
        </Group>

        {rejected.general && (
          <div className={cx(s.problem, s.pre, "selectable")} role="alert">
            {rejected.general}
          </div>
        )}
      </form>
    </Sheet>
  );
}

/** Environment variables or headers: a name and a value that behaves like an API key (AI-01, AI-29). */
function SecretRows({
  kind,
  rows,
  problems,
  onChange,
}: {
  kind: SecretKind;
  rows: SecretRow[];
  problems: (SecretRowProblem | null)[];
  onChange: (rows: SecretRow[]) => void;
}) {
  const t = useT();
  const [focusUid, setFocusUid] = useState<number | null>(null);
  const patchRow = (uid: number, p: Partial<SecretRow>) => onChange(rows.map((r) => (r.uid === uid ? { ...r, ...p } : r)));
  const add = () => {
    const row = newSecretRow();
    onChange([...rows, row]);
    setFocusUid(row.uid);
  };
  const problemText = (p: SecretRowProblem): string => {
    if (p.key === "empty") return t("aiSettings.mcp.err.keyRequired");
    if (p.key === "invalid") return t(kind === "env" ? "aiSettings.mcp.err.envKeyInvalid" : "aiSettings.mcp.err.headerKeyInvalid");
    if (p.key === "duplicate") return t("aiSettings.mcp.err.keyDuplicate");
    return t("aiSettings.mcp.err.valueRequired");
  };

  return (
    <>
      {rows.length > 0 && (
        <Group>
          <div className={s.secretTable}>
            {rows.map((row, i) => {
              const problem = problems[i];
              return (
                <div className={s.secret} key={row.uid}>
                  <div className={s.secretRow}>
                    {row.saved ? (
                      <div className={s.secretName} title={row.key}>
                        {row.key}
                      </div>
                    ) : (
                      <TextField
                        mono
                        autoFocus={focusUid === row.uid}
                        aria-label={t(kind === "env" ? "aiSettings.mcp.f.envKey" : "aiSettings.mcp.f.headerKey")}
                        value={row.key}
                        invalid={!!problem?.key}
                        placeholder={t(kind === "env" ? "aiSettings.mcp.f.envKey" : "aiSettings.mcp.f.headerKey")}
                        onChange={(e) => patchRow(row.uid, { key: e.target.value })}
                      />
                    )}
                    <SavedKeyField
                      hasSaved={row.saved}
                      draft={row.draft}
                      allowClear={false}
                      invalid={!!problem?.value}
                      ariaLabel={`${row.key} ${t("aiSettings.mcp.f.valuePlaceholder")}`.trim()}
                      placeholder={t("aiSettings.mcp.f.valuePlaceholder")}
                      newPlaceholder={t("aiSettings.mcp.f.valueNewPlaceholder")}
                      keepLabel={t("aiSettings.mcp.f.valueKeep")}
                      onChange={(draft) => patchRow(row.uid, { draft })}
                    />
                    <IconButton
                      icon="x"
                      size={13}
                      label={row.key.trim() ? t("aiSettings.mcp.f.rowRemove", { key: row.key.trim() }) : t("aiSettings.mcp.f.rowRemoveBlank")}
                      onClick={() => onChange(rows.filter((r) => r.uid !== row.uid))}
                    />
                  </div>
                  {problem && (
                    <div className={s.problem} role="alert">
                      {problemText(problem)}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </Group>
      )}
      <div>
        <Button size="sm" icon="plus" onClick={add}>
          {t(kind === "env" ? "aiSettings.mcp.f.envAdd" : "aiSettings.mcp.f.headersAdd")}
        </Button>
      </div>
    </>
  );
}
