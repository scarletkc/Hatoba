import { memo, type MouseEvent } from "react";
import { Button, Icon, IconButton, Tag } from "@hatoba/ui";
import type { TabStatus } from "@/app/tabs";
import { formatRelative, useT, type Locale } from "@/i18n";
import { isTelnetHost, type AnyHostView, type QuickTarget } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { OsIcon } from "@/lib/OsIcon";
import {
  bracketHost,
  formatTarget,
  formatTelnetTarget,
  type QuickProblem,
  type TelnetTarget,
  type TypedTarget,
} from "@/session/quickConnect";
import s from "./HostsPage.module.css";

/** Row id of the typed quick-connect target (HOST-12); saved hosts use their item ids. */
export const TYPED_ROW = "quick:typed";
/** Row id of a typed Telnet target (TELNET-05). */
export const TELNET_ROW = "quick:telnet";
export const SUGGESTIONS_ID = "recent-targets";

/** A Connect row for a target that is not a saved host: the one typed, a recent one, or a typed Telnet target. */
export type QuickRowItem =
  | {
      id: string;
      target: TypedTarget;
      telnet?: undefined;
      /** Set when the target is in the recent list, so the row can remove it. */
      recent: QuickTarget | null;
    }
  | { id: string; target?: undefined; telnet: TelnetTarget; recent: null };

export interface RowProps {
  host: AnyHostView;
  selected: boolean;
  jumpName: string | null;
  status: TabStatus | undefined;
  online: boolean | undefined;
  now: number;
  locale: Locale;
  connectLabel: string;
  failedLabel: string;
  viaTitle: string;
  onSelect: (e: MouseEvent, id: string) => void;
  onConnect: (id: string) => void;
  onContext: (e: MouseEvent, host: AnyHostView) => void;
}

export const HostRow = memo(function HostRow({
  host,
  selected,
  jumpName,
  status,
  online,
  now,
  locale,
  connectLabel,
  failedLabel,
  viaTitle,
  onSelect,
  onConnect,
  onContext,
}: RowProps) {
  const failed = status === "failed";
  const dot = status === "connected" ? "var(--green)" : failed ? "var(--red)" : online ? "var(--green)" : "var(--fg3)";
  const last = failed
    ? failedLabel
    : host.last_connected_at
      ? formatRelative(locale, host.last_connected_at, now)
      : "—";
  return (
    <div
      role="option"
      aria-selected={selected}
      data-id={host.id}
      className={cx(s.row, s.cols, selected && s.rowSel)}
      onClick={(e) => onSelect(e, host.id)}
      onDoubleClick={() => onConnect(host.id)}
      onContextMenu={(e) => onContext(e, host)}
    >
      <div className={s.nameCell}>
        {isTelnetHost(host) ? <OsIcon os={null} icon="network" badge={dot} /> : <OsIcon os={host.os} badge={dot} />}
        <span className={s.name}>{host.name}</span>
        {host.favorite && <Icon name="star" fill className={s.fav} />}
      </div>
      <div className={s.addrCell}>
        {isTelnetHost(host) ? (
          <TelnetText target={host} />
        ) : (
          <span className={s.addr}>
            {host.username && <span className={s.dim}>{host.username}@</span>}
            {host.address}
            <span className={s.dim}>:{host.port}</span>
          </span>
        )}
        {jumpName && (
          <span className={s.via} title={viaTitle}>
            <Icon name="path" />
            {jumpName}
          </span>
        )}
      </div>
      <div className={s.tags}>
        {host.tags.map((tag) => (
          <Tag key={tag} name={tag} />
        ))}
      </div>
      <div className={cx(s.last, failed && s.lastFailed)}>{last}</div>
      <div className={s.action}>
        <Button
          size="xs"
          variant="primary"
          tabIndex={-1}
          className={s.connect}
          onClick={(e) => {
            e.stopPropagation();
            onConnect(host.id);
          }}
        >
          {connectLabel}
        </Button>
      </div>
    </div>
  );
});

/** `user@host:port` in the address column's style. */
export function TargetText({ target }: { target: TypedTarget }) {
  return (
    <span className={s.addr}>
      {target.username && <span className={s.dim}>{target.username}@</span>}
      {bracketHost(target.address)}
      <span className={s.dim}>:{target.port}</span>
    </span>
  );
}

/** `telnet host:port` in the address column's style (TELNET-01). */
export function TelnetText({ target }: { target: TelnetTarget }) {
  return (
    <span className={s.addr}>
      <span className={s.dim}>telnet </span>
      {formatTelnetTarget(target)}
    </span>
  );
}

interface QuickRowProps {
  item: QuickRowItem;
  selected: boolean;
  connectLabel: string;
  hint: string;
  removeLabel: string;
  onSelect: (id: string) => void;
  onConnect: (id: string) => void;
  onRemove: (target: QuickTarget) => void;
}

/** "Connect user@host:port" for a target that is not a saved host (HOST-12). */
export function QuickRow({
  item,
  selected,
  connectLabel,
  hint,
  removeLabel,
  onSelect,
  onConnect,
  onRemove,
}: QuickRowProps) {
  const recent = item.recent;
  return (
    <div
      role="option"
      aria-selected={selected}
      data-id={item.id}
      className={cx(s.row, s.cols, selected && s.rowSel)}
      onClick={() => onSelect(item.id)}
      onDoubleClick={() => onConnect(item.id)}
    >
      <div className={s.nameCell}>
        <Icon name={recent ? "clock-counter-clockwise" : "terminal-window"} className={s.quickIcon} />
        <span className={s.name}>{connectLabel}</span>
      </div>
      <div className={s.addrCell}>
        {item.telnet ? <TelnetText target={item.telnet} /> : <TargetText target={item.target} />}
      </div>
      <div className={s.tags} />
      <div className={cx(s.last, s.quickHint)}>
        <span>{hint}</span>
        {recent && (
          <IconButton
            small
            icon="x"
            tabIndex={-1}
            className={s.forget}
            label={removeLabel}
            onClick={(e) => {
              e.stopPropagation();
              onRemove(recent);
            }}
          />
        )}
      </div>
      <div className={s.action}>
        <Button
          size="xs"
          variant="primary"
          tabIndex={-1}
          className={s.connect}
          onClick={(e) => {
            e.stopPropagation();
            onConnect(item.id);
          }}
        >
          {connectLabel}
        </Button>
      </div>
    </div>
  );
}

/** An ssh or telnet command quick connect cannot run, with the reason. It is not selectable. */
export function ProblemRow({ problem }: { problem: QuickProblem }) {
  const t = useT();
  const text =
    problem.kind === "option"
      ? t("hosts.quick.option", { option: problem.option })
      : problem.kind === "command"
        ? t("hosts.quick.command")
        : t("hosts.err.portInvalid");
  return (
    <div className={cx(s.row, s.problemRow)} role="option" aria-selected={false} aria-disabled>
      <Icon name="prohibit" className={s.quickIcon} />
      <span className={s.problemText}>{text}</span>
    </div>
  );
}

/** A note above the typed target that it connects without the forwarding it asked for. It is not selectable. */
export function NoticeRow() {
  const t = useT();
  return (
    <div className={cx(s.row, s.problemRow)} role="option" aria-selected={false} aria-disabled>
      <Icon name="info" className={s.quickIcon} />
      <span className={s.problemText}>{t("hosts.quick.noForwarding")}</span>
    </div>
  );
}

export interface SuggestionsProps {
  targets: QuickTarget[];
  active: number;
  title: string;
  removeLabel: string;
  onHover: (index: number) => void;
  onConnect: (target: QuickTarget) => void;
  onRemove: (target: QuickTarget) => void;
}

/** Recent quick-connect targets under the empty search field (HOST-12). */
export function RecentSuggestions({
  targets,
  active,
  title,
  removeLabel,
  onHover,
  onConnect,
  onRemove,
}: SuggestionsProps) {
  return (
    // Clicks inside keep the focus in the search field, so the list stays open.
    <div className={s.suggest} onMouseDown={(e) => e.preventDefault()}>
      <div className={s.suggestTitle}>{title}</div>
      <div id={SUGGESTIONS_ID} role="listbox" aria-label={title}>
        {targets.map((target, i) => (
          <div
            key={formatTarget(target)}
            id={`${SUGGESTIONS_ID}-${i}`}
            role="option"
            aria-selected={i === active}
            className={cx(s.suggestItem, i === active && s.suggestItemActive)}
            onMouseEnter={() => onHover(i)}
            onClick={() => onConnect(target)}
          >
            <Icon name="clock-counter-clockwise" className={s.quickIcon} />
            <span className={s.suggestLabel}>
              <TargetText target={target} />
            </span>
            <IconButton
              small
              icon="x"
              tabIndex={-1}
              className={s.forget}
              label={removeLabel}
              onClick={(e) => {
                e.stopPropagation();
                onRemove(target);
              }}
            />
          </div>
        ))}
      </div>
    </div>
  );
}

export interface QuickRowsListProps {
  problem: QuickProblem | null;
  notice: boolean;
  quickRows: readonly QuickRowItem[];
  selectedSet: ReadonlySet<string>;
  onSelect: (id: string) => void;
  onConnect: (id: string) => void;
  onRemoveRecent: (target: QuickTarget) => void;
}

export function QuickRowsList({
  problem,
  notice,
  quickRows,
  selectedSet,
  onSelect,
  onConnect,
  onRemoveRecent,
}: QuickRowsListProps) {
  const t = useT();
  return (
    <>
      {problem && <ProblemRow problem={problem} />}
      {notice && <NoticeRow />}
      {quickRows.map((r) => (
        <QuickRow
          key={r.id}
          item={r}
          selected={selectedSet.has(r.id)}
          connectLabel={t("hosts.connect")}
          hint={r.recent ? t("hosts.quick.recent") : t("hosts.quick.notSaved")}
          removeLabel={t("hosts.quick.forget")}
          onSelect={onSelect}
          onConnect={onConnect}
          onRemove={onRemoveRecent}
        />
      ))}
    </>
  );
}
