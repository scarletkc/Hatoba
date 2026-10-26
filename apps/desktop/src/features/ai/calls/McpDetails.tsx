import { Chip } from "@hatoba/ui";
import { useT, type MessageKey } from "@/i18n";
import type { McpToolAnnotations, McpToolInfo } from "@/ipc/types";
import { cx } from "@/lib/cx";
import s from "../messages/Messages.module.css";

/** MCP annotations as chips (AI-31): shown, but they never change whether a call asks. */
const HINTS: { key: Exclude<keyof McpToolAnnotations, "title">; yes: MessageKey; no: MessageKey; warn?: boolean }[] = [
  { key: "read_only_hint", yes: "ai.annot.readOnly", no: "ai.annot.notReadOnly" },
  { key: "destructive_hint", yes: "ai.annot.destructive", no: "ai.annot.notDestructive", warn: true },
  { key: "idempotent_hint", yes: "ai.annot.idempotent", no: "ai.annot.notIdempotent" },
  { key: "open_world_hint", yes: "ai.annot.openWorld", no: "ai.annot.closedWorld" },
];

export function McpDetails({ mcp }: { mcp: McpToolInfo }) {
  const t = useT();
  const { annotations: a, description } = mcp.tool;
  const hints = HINTS.filter((h) => a[h.key] !== null);
  return (
    <>
      {a.title && a.title !== mcp.tool.tool && <div className={s.mcpTitle}>{a.title}</div>}
      {description.trim() && (
        <>
          <div className={s.label}>{t("ai.approval.description")}</div>
          <div className={cx(s.description, "selectable")}>{description}</div>
        </>
      )}
      {hints.length > 0 && (
        <div className={s.hints} title={t("ai.approval.hints")} aria-label={t("ai.approval.hints")}>
          {hints.map((h) => (
            <Chip key={h.key} tone={h.warn && a[h.key] ? "error" : "neutral"}>
              {t(a[h.key] ? h.yes : h.no)}
            </Chip>
          ))}
        </div>
      )}
    </>
  );
}
