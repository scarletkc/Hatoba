import { useEffect, useId, useRef, useState } from "react";
import { errorMessage } from "@/app/errors";
import { Button, Icon, TextArea } from "@/components/controls";
import { Group, Section } from "@/components/layout";
import { FooterSpacer, Sheet, SheetHeader, toast } from "@/components/overlay";
import { useT, type MessageKey } from "@/i18n";
import { api, toAppError } from "@/ipc/api";
import type { McpImportPreview, McpServerView } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { transportSummary } from "./mcpLogic";
import s from "./McpSection.module.css";

/** Reads a file the user picks in the page itself: the JSON is sent to the backend as text, so no path is needed. */
function readJsonFile(): Promise<string | null> {
  return new Promise((resolve, reject) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = ".json,.jsonc,application/json";
    input.onchange = () => {
      const file = input.files?.[0];
      if (!file) return resolve(null);
      file.text().then(resolve, reject);
    };
    input.oncancel = () => resolve(null);
    input.click();
  });
}

/**
 * Import MCP servers from the JSON other clients keep them in (AI-33): paste it or pick the file, review what would be
 * added and what is skipped, then import. Environment variable and header values move into the vault; the text is not
 * kept once the dialog closes.
 */
export function McpImportDialog({ onClose, onImported }: { onClose: () => void; onImported: (added: McpServerView[]) => void }) {
  const t = useT();
  const ids = { form: useId(), text: useId() };
  const [text, setText] = useState("");
  const [preview, setPreview] = useState<McpImportPreview | null>(null);
  const [busy, setBusy] = useState<"preview" | "import" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);

  const chooseFile = async () => {
    try {
      const contents = await readJsonFile();
      if (contents === null) return;
      setText(contents);
      setError(null);
    } catch {
      setError(t("aiSettings.mcp.imp.fileFailed"));
    }
  };

  /** The reason an import was refused: the backend names the problem in the JSON (a line, a missing key). */
  const failure = (e: unknown): string => {
    const err = toAppError(e);
    return err.code === "invalid_input" && err.detail ? err.detail : errorMessage(t, e);
  };

  const runPreview = async () => {
    if (busy || !text.trim()) return;
    setBusy("preview");
    setError(null);
    try {
      const got = await api.mcp_import_preview(text);
      if (live.current) setPreview(got);
    } catch (e) {
      if (live.current) setError(failure(e));
    } finally {
      if (live.current) setBusy(null);
    }
  };

  const runImport = async () => {
    if (busy || !preview || preview.servers.length === 0) return;
    setBusy("import");
    setError(null);
    try {
      const added = await api.mcp_import(text);
      toast(t("aiSettings.mcp.imp.done", { n: added.length }), "success");
      onImported(added);
      onClose();
    } catch (e) {
      if (!live.current) return;
      setError(failure(e));
      setBusy(null);
    }
  };

  const count = preview?.servers.length ?? 0;
  const hasStdio = !!preview?.servers.some((x) => x.transport.kind === "stdio");

  return (
    <Sheet
      width={680}
      onClose={busy ? undefined : onClose}
      closeOnBackdrop={false}
      footer={
        <>
          {preview && (
            <Button
              onClick={() => {
                setPreview(null);
                setError(null);
              }}
              disabled={!!busy}
            >
              {t("aiSettings.mcp.imp.back")}
            </Button>
          )}
          <FooterSpacer />
          <Button onClick={onClose} disabled={!!busy}>
            {t("btn.cancel")}
          </Button>
          {preview ? (
            <Button variant="primary" type="submit" form={ids.form} busy={busy === "import"} disabled={count === 0}>
              {t("aiSettings.mcp.imp.submit", { n: count })}
            </Button>
          ) : (
            <Button variant="primary" type="submit" form={ids.form} busy={busy === "preview"} disabled={!text.trim()}>
              {t("aiSettings.mcp.imp.preview")}
            </Button>
          )}
        </>
      }
    >
      <SheetHeader title={t("aiSettings.mcp.imp.title")} subtitle={t("aiSettings.mcp.imp.subtitle")} />
      <form
        id={ids.form}
        className={s.form}
        onSubmit={(e) => {
          e.preventDefault();
          void (preview ? runImport() : runPreview());
        }}
      >
        {!preview && (
          <div className={s.stack}>
            <div className={s.pasteBar}>
              <label className={s.pasteLabel} htmlFor={ids.text}>
                {t("aiSettings.mcp.imp.paste")}
              </label>
              <Button size="sm" icon="folder-open" onClick={() => void chooseFile()}>
                {t("aiSettings.mcp.imp.file")}
              </Button>
            </div>
            <TextArea
              id={ids.text}
              className={s.json}
              rows={10}
              autoFocus
              value={text}
              aria-invalid={!!error || undefined}
              placeholder={t("aiSettings.mcp.imp.placeholder")}
              onChange={(e) => {
                setText(e.target.value);
                setError(null);
              }}
            />
          </div>
        )}

        {preview && (
          <>
            {count === 0 && <div className={s.note}>{t("aiSettings.mcp.imp.none")}</div>}
            {count > 0 && (
              <Section title={t("aiSettings.mcp.imp.found")}>
                <Group>
                  {preview.servers.map((server) => (
                    <div className={s.found} key={server.name}>
                      <div className={s.foundHead}>
                        <span className={s.foundName}>{server.name}</span>
                        <span className={s.badge}>{t(`aiSettings.mcp.kind.${server.transport.kind}` as MessageKey)}</span>
                      </div>
                      <div className={cx(s.commandLine, s.commandLineRow, "selectable")}>{transportSummary(server.transport)}</div>
                      {(server.transport.kind === "stdio" ? server.transport.env_keys : server.transport.header_keys).length > 0 && (
                        <div className={s.hint}>
                          {t(server.transport.kind === "stdio" ? "aiSettings.mcp.imp.envNames" : "aiSettings.mcp.imp.headerNames", {
                            names: (server.transport.kind === "stdio" ? server.transport.env_keys : server.transport.header_keys).join(", "),
                          })}
                        </div>
                      )}
                      {server.exists && (
                        <div className={cx(s.hint, s.hintWarn)}>
                          <Icon name="warning" /> {t("aiSettings.mcp.imp.exists")}
                        </div>
                      )}
                    </div>
                  ))}
                </Group>
                {hasStdio && <div className={s.hint}>{t("aiSettings.mcp.imp.stdioNote")}</div>}
              </Section>
            )}
            {preview.skipped.length > 0 && (
              <Section title={t("aiSettings.mcp.imp.skipped")}>
                <Group>
                  {preview.skipped.map((entry, i) => (
                    <div className={s.found} key={`${entry.name}-${i}`}>
                      <div className={s.foundName}>{entry.name}</div>
                      <div className={cx(s.hint, "selectable")}>{entry.reason}</div>
                    </div>
                  ))}
                </Group>
              </Section>
            )}
          </>
        )}

        {error && (
          <div className={cx(s.problem, s.pre, "selectable")} role="alert">
            {error}
          </div>
        )}
      </form>
    </Sheet>
  );
}
