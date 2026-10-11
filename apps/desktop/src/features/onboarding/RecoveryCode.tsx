import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage } from "@/app/errors";
import { LinkButton, TextField } from "@/components/controls";
import { toast } from "@/components/overlay";
import { formatDate, useT } from "@/i18n";
import { api } from "@/ipc/api";
import { copyText, pickSavePath } from "@/lib/native";
import s from "./RecoveryCode.module.css";

/** Split a formatted recovery code ("K7QF-2M9X-…") into its display groups. */
export function codeGroups(code: string): string[] {
  const parts = code.split("-").filter(Boolean);
  if (parts.length > 1) return parts;
  return code.match(/.{1,4}/g) ?? [];
}

/** Crockford base32 is forgiving: O reads as 0, I and L read as 1. */
function normalize(input: string): string {
  return input.toUpperCase().replace(/[^0-9A-Z]/g, "").replace(/O/g, "0").replace(/[IL]/g, "1");
}

/**
 * The recovery code as 8 groups with copy / save-as-text / print actions (design §05 step 3), plus the
 * VAULT-02 confirmation: retype the last group. `onConfirmedChange` reports whether it matches.
 */
export function RecoveryCodeSection({ code, onConfirmedChange }: { code: string; onConfirmedChange: (ok: boolean) => void }) {
  const t = useT();
  const groups = codeGroups(code);
  const last = groups[groups.length - 1] ?? "";
  const [copied, setCopied] = useState(false);
  const [lastInput, setLastInput] = useState("");

  const lastOk = normalize(lastInput) === normalize(last) && lastInput.trim() !== "";
  const lastWrong = !lastOk && normalize(lastInput).length >= normalize(last).length;

  useEffect(() => onConfirmedChange(lastOk), [lastOk, onConfirmedChange]);
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1600);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    try {
      await copyText(code);
      setCopied(true);
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  const saveText = async () => {
    try {
      const path = await pickSavePath("hatoba-recovery-code.txt", { name: "Text", extension: "txt" });
      if (!path) return;
      const text = `${t("vault.code.fileHeader")}\n\n${code}\n\n${t("vault.code.fileNote")}\n${formatDate(Date.now())}\n`;
      await api.save_text_file(path, text);
      toast(t("vault.code.saved"), "success");
    } catch (e) {
      toast(errorMessage(t, e), "error");
    }
  };

  return (
    <>
      <div className={s.block}>
        <div className={s.head}>
          <span className={s.title}>{t("vault.code.label")}</span>
          <LinkButton icon="copy" onClick={() => void copy()}>
            {copied ? t("btn.copied") : t("btn.copy")}
          </LinkButton>
          <LinkButton icon="download-simple" onClick={() => void saveText()}>
            {t("vault.code.save")}
          </LinkButton>
          <LinkButton icon="printer" onClick={() => window.print()}>
            {t("vault.code.print")}
          </LinkButton>
        </div>
        <div className={s.grid} role="group" aria-label={t("vault.code.label")}>
          {groups.map((g, i) => (
            <span key={i} className={s.item}>
              <span className={s.index}>{i + 1}</span>
              <span className={s.group}>{g}</span>
            </span>
          ))}
        </div>
      </div>

      <div className={s.block}>
        <label className={s.confirmLabel} htmlFor="recovery-last">
          {t("vault.code.confirmLast")}
        </label>
        <div className={s.confirmField}>
          <TextField
            id="recovery-last"
            mono
            large
            autoComplete="off"
            aria-label={t("vault.code.lastGroup")}
            placeholder={"·".repeat(last.length)}
            invalid={lastWrong}
            maxLength={last.length + 4}
            value={lastInput}
            onChange={(e) => setLastInput(e.target.value.toUpperCase())}
          />
        </div>
        {lastWrong && (
          <div className={s.error} role="alert">
            {t("vault.code.lastMismatch")}
          </div>
        )}
      </div>

      <PrintableCode groups={groups} />
    </>
  );
}

/** Hidden on screen; `window.print()` shows only this (see the print rules in the CSS module). */
function PrintableCode({ groups }: { groups: string[] }) {
  const t = useT();
  return createPortal(
    <div className={s.print} aria-hidden>
      <h1>{t("vault.code.fileHeader")}</h1>
      <div className={s.printCode}>
        {groups.map((g, i) => (
          <span key={i}>
            <small>{i + 1}</small> {g}
          </span>
        ))}
      </div>
      <p>{t("vault.code.fileNote")}</p>
      <p>{formatDate(Date.now())}</p>
    </div>,
    document.body,
  );
}
