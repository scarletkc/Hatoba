import { Fragment, useMemo } from "react";
import { Chip, Icon } from "@hatoba/ui";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { showTerminalInput } from "../terminal/sendKeys";
import { shownText } from "../tools";
import s from "../messages/Messages.module.css";

/**
 * A call's input on the approval card (AI-17), all of it: no scroll fold, every line break marked (a final
 * one presses Enter), the line count when there are several, and hidden characters (controls, bidirectional
 * overrides) as escapes, with a note that there are some.
 */
export function ShownInput({ text, command, terminal }: { text: string; command?: boolean; terminal?: boolean }) {
  const t = useT();
  const shown = useMemo(() => {
    const input = terminal ? showTerminalInput(text) : text;
    const shown = shownText(input);
    return { ...shown, escaped: shown.escaped || input !== text };
  }, [text, terminal]);
  const multiline = shown.lines.length > 1;
  return (
    <>
      {(multiline || shown.escaped) && (
        <div className={s.inputNotes}>
          {multiline && <Chip tone="warning">{t("ai.approval.lines", { n: shown.lines.length })}</Chip>}
          {shown.escaped && <Chip tone="warning">{t("ai.approval.escaped")}</Chip>}
        </div>
      )}
      <pre className={cx(s.pre, s.whole, command && s.command, "selectable")}>
        {shown.lines.map((line, i) => (
          <Fragment key={i}>
            {line}
            {i < shown.lines.length - 1 && (
              <>
                <span className={s.lineBreak} aria-hidden>
                  ↵
                </span>
                {"\n"}
              </>
            )}
          </Fragment>
        ))}
      </pre>
    </>
  );
}

/**
 * The directory a `run_command` call starts in, on its approval card (AI-17): the whole path, wrapped rather than
 * cut, with hidden characters as escapes.
 */
export function Workdir({ path }: { path: string }) {
  const t = useT();
  const shown = useMemo(() => shownText(path), [path]);
  return (
    <div className={s.workdir}>
      <Icon name="folder-simple" size={12} className={s.workdirIcon} />
      <span className={s.workdirLabel}>{t("ai.approval.workdir")}</span>
      <span className={cx(s.workdirPath, "selectable")}>{shown.lines.join("↵")}</span>
      {shown.escaped && <Chip tone="warning">{t("ai.approval.escaped")}</Chip>}
    </div>
  );
}
