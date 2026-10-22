import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode, type RefObject } from "react";
import { createPortal } from "react-dom";
import { create } from "zustand";
import { useTabs } from "@/app/tabs";
import { Icon, Spinner } from "@/components/controls";
import { overlayStyles, useExitAnimation, useOverlaysLocked, windowSurface } from "@/components/overlay";
import { Chip } from "@/components/patterns";
import { useT } from "@/i18n";
import { api } from "@/ipc/api";
import type { AiHostListing, AiSkillCommand } from "@/ipc/types";
import { cx } from "@/lib/cx";
import { isImeEvent } from "@/lib/ime";
import { attachHostFile, attachHostFolder, attachResource, hostRefusal } from "./commandActions";
import {
  commandItems,
  listPath,
  mentionText,
  pathItems,
  replaceTrigger,
  resourceItems,
  resourceTarget,
  runsAtOnce,
  splitPath,
  triggerAt,
  type BuiltinName,
  type CommandItem,
  type Trigger,
} from "./commands";
import { loadMcp, menuSources, useMcp } from "./mcp";
import { getSlot, type Slot } from "./store";
import s from "./CommandMenu.module.css";

/** At most this many rows; typing more narrows them. */
const MAX_ROWS = 60;

const BUILTIN_ICON: Record<BuiltinName, string> = {
  btw: "chat-teardrop-dots",
  clear: "note-pencil",
  compact: "arrows-in-line-vertical",
  mcp: "plug",
  memory: "brain",
  model: "cube",
};

/** The skills the `/` menu and the + menu's Skills offer (AI-50), read again each time either opens. */
export const useSkills = create<{ skills: AiSkillCommand[] | null }>(() => ({ skills: null }));

export function loadSkills() {
  api.ai_skill_commands().then(
    (skills) => useSkills.setState({ skills }),
    () => useSkills.setState((st) => ({ skills: st.skills ?? [] })),
  );
}

/** One row of the menu. Rows without `pick` say something (loading, a failure) and cannot be picked. */
interface Row {
  key: string;
  icon: string;
  label: string;
  /** What follows a command's name, such as `<question>`. */
  args?: string;
  description?: string;
  /** A short tag at the end, such as "Skill" or a server's name. */
  badge?: string;
  /** The label is text, not a command or a path. */
  plain?: boolean;
  busy?: boolean;
  /** Enter, or `tab` for Tab, which completes rather than runs. */
  pick?: (how: "enter" | "tab") => void;
}

export interface CommandMenu {
  /** The menu, rendered in a portal above the input box; null while closed. */
  element: ReactNode;
  /** Handles the keys the open menu takes (↑, ↓, Enter, Tab, Esc); true when it took the key. */
  onKeyDown: (e: KeyboardEvent<HTMLTextAreaElement>) => boolean;
  /** The caret moved, or the text changed. */
  onCaret: () => void;
  /** The box lost the focus: the menu closes. */
  onBlur: () => void;
  /** The textarea's combobox attributes. */
  inputProps: Record<string, string | boolean | undefined>;
  /** An `@` pick is being attached: the message waits for it, so that it does not go without it. */
  busy: boolean;
}

/**
 * AI-50, AI-51: the menu that opens on `/` at the start of the input box and on `@` at the start of a word. It filters
 * as the user types; ↑ and ↓ move, Enter and Tab pick, and Esc closes it until the word changes. The Enter that
 * commits an input method's candidate never picks. Picking a command completes its name, and Enter runs one that
 * needs nothing after its name (`run`); picking a mention attaches it and leaves `@` and its name in the text.
 */
export function useCommandMenu({
  slotId,
  slot,
  input,
  box,
  setDraft,
  run,
  onLocalFiles,
}: {
  slotId: string;
  slot: Slot;
  input: RefObject<HTMLTextAreaElement | null>;
  box: RefObject<HTMLDivElement | null>;
  /** Puts `text` into the box with the caret at `caret`, giving the box the focus when `focus`. */
  setDraft: (text: string, caret: number, focus: boolean) => void;
  /** Runs the command the box holds, as Enter in the box would. */
  run: (text: string) => void;
  /** Opens the file picker for text files from this computer (AI-35). */
  onLocalFiles: () => void;
}): CommandMenu {
  const t = useT();
  const [caret, setCaret] = useState<{ start: number; end: number } | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [active, setActive] = useState(0);
  const [attaching, setAttaching] = useState(false);
  // Read by picks whose closures a memo keeps, so a second pick while one attaches does nothing.
  const attachingRef = useRef(false);
  const draft = slot.draft;
  const trigger = caret ? triggerAt(draft, caret.start, caret.end) : null;
  const triggerKey = trigger ? `${trigger.sign}${trigger.start}` : null;
  const open = !!trigger && dismissed !== triggerKey;

  const session = useTabs((st) => {
    const tab = st.tabs.find((x) => x.id === slotId);
    return tab?.status === "connected" ? (tab.sessionId ?? null) : null;
  });
  const skills = useSkills((st) => st.skills);
  const servers = useMcp((st) => st.servers);
  const status = useMcp((st) => st.status);
  const sources = useMemo(() => menuSources(servers ?? [], status, slot.mcpOff), [servers, status, slot.mcpOff]);

  // Each opening reads the skills and starts the MCP servers the conversation uses, for their prompts and resources.
  const sign = open ? trigger.sign : null;
  useEffect(() => {
    if (!sign) return;
    if (sign === "/") loadSkills();
    void loadMcp();
    void api.mcp_warm(slot.mcpOff).catch(() => {});
    // Once per opening; a server switched off meanwhile is left out by `menuSources`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sign, slotId]);

  // A closed trigger opens again once it changes.
  useEffect(() => {
    if (dismissed && dismissed !== triggerKey) setDismissed(null);
  }, [dismissed, triggerKey]);

  // The host's directory listings, by session and path, while the menu is open. A listing that answers after the
  // menu closed, or after the tab changed, is dropped.
  const [listings, setListings] = useState<Record<string, AiHostListing | "loading">>({});
  const listingGen = useRef(0);

  // Another tab's conversation starts with a closed menu, and nothing of this one's.
  useEffect(() => {
    setCaret(null);
    setDismissed(null);
    setListings({});
    listingGen.current++;
  }, [slotId]);
  const pathQuery = open && trigger.sign === "@" && session ? splitPath(trigger.query) : null;
  const dirKey = pathQuery && session ? `${session}\n${listPath(pathQuery.dir)}` : null;
  useEffect(() => {
    if (!dirKey || listings[dirKey] || !session || !pathQuery) return;
    setListings((l) => ({ ...l, [dirKey]: "loading" }));
    const gen = listingGen.current;
    const store = (listing: AiHostListing) => {
      if (listingGen.current === gen) setListings((l) => ({ ...l, [dirKey]: listing }));
    };
    api.ai_host_list(session, listPath(pathQuery.dir)).then(store, () => store({ status: "refused", reason: "failed" }));
  }, [dirKey, listings, session, pathQuery]);
  // A new opening lists again, so a file created since shows.
  useEffect(() => {
    if (open) return;
    setListings({});
    listingGen.current++;
  }, [open]);

  const replace = (tr: Trigger, insert: string, focus = true) => {
    const next = replaceTrigger(draft, tr, insert);
    setDraft(next.text, next.caret, focus);
  };

  /**
   * Attaches what an `@` row names, then leaves `@` and its name in the text, unless the text changed meanwhile. The box
   * takes the focus back only when it still has it: a slow read may end after the user moved on.
   */
  const attach = (tr: Trigger, target: string, work: () => Promise<boolean>) => {
    if (attachingRef.current) return;
    attachingRef.current = true;
    setAttaching(true);
    const before = draft;
    void work()
      .then((ok) => {
        if (ok && getSlot(slotId).draft === before) replace(tr, mentionText(target), document.activeElement === input.current);
      })
      .finally(() => {
        attachingRef.current = false;
        setAttaching(false);
      });
  };

  const rows: Row[] = useMemo(() => {
    if (!open) return [];
    if (trigger.sign === "/") {
      const items = commandItems(trigger.query, skills ?? [], sources.prompts);
      const list: Row[] = items.map((item) => commandRow(item, (how) => pickCommand(item, how)));
      if (skills === null) list.push({ key: "loading", icon: "", label: t("ai.cmd.loading"), busy: true });
      else if (list.length === 0) list.push({ key: "none", icon: "info", label: t("ai.cmd.none") });
      return list;
    }
    const host: Row[] = [];
    if (session && pathQuery && dirKey) {
      const listing = listings[dirKey];
      const shownDir = pathQuery.dir || "~/";
      if (!listing || listing === "loading") host.push({ key: "listing", icon: "", label: t("ai.mention.listing", { path: shownDir }), busy: true });
      else if (listing.status === "refused") {
        if (listing.reason !== "not_found" || pathQuery.dir) host.push({ key: "refused", icon: "warning-circle", label: hostRefusal(listing.reason, shownDir, "list") });
      } else {
        if (!pathQuery.leaf)
          host.push({
            key: "folder",
            icon: "folder-open",
            label: shownDir,
            description: t("ai.mention.folder", { n: listing.entries.length }),
            pick: () => attach(trigger, pathQuery.dir || "~/", () => attachHostFolder(slotId, session, listPath(pathQuery.dir))),
          });
        for (const entry of pathItems(listing.entries, pathQuery.leaf)) {
          const path = `${pathQuery.dir}${entry.name}`;
          host.push(
            entry.dir
              ? { key: `d:${entry.name}`, icon: "folder", label: `${entry.name}/`, pick: () => replace(trigger, trigger.quoted || /\s/.test(path) ? `@"${path}/` : `@${path}/`) }
              : { key: `f:${entry.name}`, icon: "file-text", label: entry.name, pick: () => attach(trigger, path, () => attachHostFile(slotId, session, path)) },
          );
        }
        if (listing.cut) host.push({ key: "cut", icon: "info", label: t("ai.mention.cut") });
      }
    }
    const resources: Row[] = resourceItems(sources.resources, trigger.query).map((item) => ({
      key: `r:${item.serverId}:${item.resource.uri}`,
      icon: "database",
      label: item.resource.title || item.resource.name,
      plain: true,
      description: item.resource.uri,
      badge: item.server,
      pick: () => attach(trigger, resourceTarget(item), () => attachResource(slotId, item)),
    }));
    const local: Row[] = trigger.query
      ? []
      : [
          {
            key: "local",
            icon: "paperclip",
            label: t("ai.mention.local"),
            plain: true,
            pick: () => {
              replace(trigger, "");
              onLocalFiles();
            },
          },
        ];
    // A large directory never pushes the resources and the files from this computer out of the menu: the host's rows
    // get what the resources leave, and at least half.
    const room = MAX_ROWS - local.length;
    const hostRoom = Math.max(Math.ceil(room / 2), room - resources.length);
    const list = [...[...host.slice(0, hostRoom), ...resources].slice(0, room), ...local];
    if (list.length === 0) list.push({ key: "none", icon: "info", label: session ? t("ai.mention.none") : t("ai.mention.noTerminal") });
    return list;
    // The pick closures read the draft through `replace` at pick time.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, trigger?.sign, trigger?.query, trigger?.start, skills, sources, session, listings, dirKey, draft, t]);

  function commandRow(item: CommandItem, pick: (how: "enter" | "tab") => void): Row {
    switch (item.kind) {
      case "builtin":
        return {
          key: `b:${item.name}`,
          icon: BUILTIN_ICON[item.command.name],
          label: `/${item.name}`,
          args: item.command.args ? t(item.command.args) : undefined,
          description: t(item.command.description),
          pick,
        };
      case "skill":
        return { key: `s:${item.name}`, icon: "book-open-text", label: `/${item.name}`, description: item.skill.description, badge: t("ai.cmd.skill"), pick };
      case "prompt": {
        const p = item.prompt.prompt;
        const args = p.arguments.map((a) => (a.required ? `<${a.name}>` : `[${a.name}]`)).join(" ");
        return { key: `p:${item.name}`, icon: "chat-text", label: `/${item.name}`, args: args || undefined, description: p.description || p.title || undefined, badge: item.prompt.server, pick };
      }
    }
  }

  function pickCommand(item: CommandItem, how: "enter" | "tab") {
    if (!trigger) return;
    const text = `/${item.name}`;
    if (how === "enter" && runsAtOnce(item)) {
      run(replaceTrigger(draft, trigger, text).text);
      return;
    }
    replace(trigger, `${text} `);
  }

  const pickable = rows.filter((r) => r.pick);
  const current = pickable.length ? Math.min(active, pickable.length - 1) : -1;
  useEffect(() => setActive(0), [triggerKey, trigger?.query]);

  const onCaret = () => {
    const el = input.current;
    if (el) setCaret({ start: el.selectionStart, end: el.selectionEnd });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!open) return false;
    // While a pick attaches, the menu keeps its keys, so Enter does not send meanwhile.
    if (attachingRef.current && (e.key === "Enter" || e.key === "Tab")) {
      e.preventDefault();
      return true;
    }
    // The Enter that commits an input method's candidate never picks or sends.
    if (isImeEvent(e)) return false;
    const plain = !e.shiftKey && !e.altKey && !e.ctrlKey && !e.metaKey;
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      setDismissed(triggerKey);
      return true;
    }
    if (!plain || pickable.length === 0) return false;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = pickable.length;
      setActive((current + (e.key === "ArrowDown" ? 1 : n - 1)) % n);
      return true;
    }
    if (e.key === "Enter" || e.key === "Tab") {
      e.preventDefault();
      pickable[current]?.pick?.(e.key === "Tab" ? "tab" : "enter");
      return true;
    }
    return false;
  };

  const listId = `ai-cmd-${slotId}`;
  const activeRow = pickable[current];
  const element = open ? (
    <MenuPopover
      box={box}
      id={listId}
      label={trigger.sign === "/" ? t("ai.cmd.menu") : t("ai.mention.menu")}
      rows={rows}
      activeKey={activeRow?.key ?? null}
      busy={attaching}
      onHover={(key) => {
        const i = pickable.findIndex((r) => r.key === key);
        if (i >= 0) setActive(i);
      }}
      onPick={(row) => row.pick?.("enter")}
    />
  ) : null;

  return {
    element,
    onKeyDown,
    onCaret,
    onBlur: () => setCaret(null),
    busy: attaching,
    inputProps: {
      role: "combobox",
      "aria-expanded": open,
      "aria-controls": open ? listId : undefined,
      "aria-autocomplete": "list",
      "aria-activedescendant": open && activeRow ? rowId(listId, activeRow.key) : undefined,
    },
  };
}

/** A row's element id: its key holds file names, which may have spaces, and an id may not. */
function rowId(listId: string, key: string): string {
  return `${listId}-${encodeURIComponent(key)}`;
}

/** The menu's list above the input box, as wide as the box. */
function MenuPopover({
  box,
  id,
  label,
  rows,
  activeKey,
  busy,
  onHover,
  onPick,
}: {
  box: RefObject<HTMLDivElement | null>;
  id: string;
  label: string;
  rows: Row[];
  activeKey: string | null;
  busy: boolean;
  onHover: (key: string) => void;
  onPick: (row: Row) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const locked = useOverlaysLocked();
  const [pos, setPos] = useState<{ left: number; bottom: number; width: number; maxHeight: number } | null>(null);
  useExitAnimation(ref, !locked && !!pos);

  useLayoutEffect(() => {
    const place = () => {
      const r = box.current?.getBoundingClientRect();
      if (!r) return;
      const b = windowSurface();
      setPos({ left: r.left, width: r.width, bottom: window.innerHeight - r.top + 6, maxHeight: Math.max(120, Math.min(360, r.top - b.top - 14)) });
    };
    place();
    if (typeof ResizeObserver === "undefined" || !box.current) return;
    const observer = new ResizeObserver(place);
    observer.observe(box.current);
    window.addEventListener("resize", place);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", place);
    };
  }, [box]);

  // The active row stays in view as ↑ and ↓ move it.
  useEffect(() => {
    if (!activeKey) return;
    ref.current?.querySelector<HTMLElement>(`[data-key="${CSS.escape(activeKey)}"]`)?.scrollIntoView({ block: "nearest" });
  }, [activeKey]);

  if (locked || !pos) return null;
  return createPortal(
    // The box keeps the focus when the list's padding or scroll bar is clicked too.
    <div ref={ref} id={id} role="listbox" aria-label={label} aria-busy={busy} className={cx(s.menu, overlayStyles.popIn)} style={pos} onMouseDown={(e) => e.preventDefault()}>
      {rows.map((row) => {
        const selected = row.key === activeKey;
        return (
          <div
            key={row.key}
            id={row.pick ? rowId(id, row.key) : undefined}
            data-key={row.key}
            role={row.pick ? "option" : "presentation"}
            aria-selected={row.pick ? selected : undefined}
            className={cx(s.row, selected && s.active, !row.pick && s.info, row.plain && s.plain)}
            onMouseMove={() => row.pick && onHover(row.key)}
            onClick={() => row.pick && onPick(row)}
          >
            {row.busy ? <Spinner size={12} /> : row.icon && <Icon name={row.icon} size={14} className={s.icon} />}
            <span className={s.main}>
              <span className={s.label}>{row.label}</span>
              {row.args && <span className={s.args}>{row.args}</span>}
            </span>
            {row.description && <span className={s.description}>{row.description}</span>}
            {row.badge && (
              <Chip className={s.badge}>
                <span className={s.badgeText}>{row.badge}</span>
              </Chip>
            )}
          </div>
        );
      })}
    </div>,
    document.body,
  );
}
