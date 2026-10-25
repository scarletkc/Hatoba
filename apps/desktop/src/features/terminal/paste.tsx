import { confirm } from "@hatoba/ui";
import { t } from "@/i18n";
import s from "./paste.module.css";

const PREVIEW_LINES = 6;
const PREVIEW_COLUMNS = 90;

/** TERM-04: ask before pasting text that contains line breaks. Resolves true when the user agrees. */
export function confirmMultilinePaste(text: string): Promise<boolean> {
  const lines = text.split(/\r\n|\r|\n/);
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop(); // a final newline isn't another line
  const shown = lines
    .slice(0, PREVIEW_LINES)
    .map((l) => (l.length > PREVIEW_COLUMNS ? `${l.slice(0, PREVIEW_COLUMNS)}…` : l));
  const hidden = lines.length - shown.length;
  return confirm({
    title: t("terminal.paste.title"),
    icon: "clipboard-text",
    body: (
      <>
        <div>{t("terminal.paste.body", { n: lines.length })}</div>
        <pre className={`${s.preview} selectable`}>
          {shown.join("\n")}
          {hidden > 0 && <span className={s.more}>{`\n${t("terminal.paste.more", { n: hidden })}`}</span>}
        </pre>
      </>
    ),
    confirmLabel: t("terminal.paste.confirm"),
  });
}
