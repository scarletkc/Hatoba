import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { errorMessage } from "@/app/errors";
import { Checkbox, LinkButton } from "@/components/controls";
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

/**
 * The recovery code as 8 groups with copy / save-as-text / print actions (design §05 step 3), plus the
 * VAULT-02 confirmation: tick "I've saved it". `onConfirmedChange` reports whether it is ticked.
 */
export function RecoveryCodeSection({ code, onConfirmedChange }: { code: string; onConfirmedChange: (ok: boolean) => void }) {
  const t = useT();
  const groups = codeGroups(code);
  const [copied, setCopied] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => onConfirmedChange(saved), [saved, onConfirmedChange]);
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
          {/* A check rather than "Copied", which would push the links after it along. */}
          <LinkButton icon={copied ? "check" : "copy"} aria-label={copied ? t("btn.copied") : undefined} onClick={() => void copy()}>
            {t("btn.copy")}
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

      <Checkbox checked={saved} onChange={setSaved}>
        {t("vault.code.savedCheck")}
      </Checkbox>

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
