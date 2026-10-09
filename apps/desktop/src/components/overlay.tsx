import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";
import { create } from "zustand";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import { Button, Icon, controlStyles } from "./controls";
import o from "./overlay.module.css";

// ───────────────────────── Lock ─────────────────────────

// Overlays render into document.body, outside the app that the lock screen hides (SEC-03), so they
// hide themselves while the vault is locked.
const useLockStore = create<{ locked: boolean }>(() => ({ locked: false }));

export const useOverlaysLocked = () => useLockStore((st) => st.locked);

/** Hides modals and closes menus while the vault is locked, and cancels a pending `confirm()`. */
export function setOverlaysLocked(locked: boolean) {
  if (useLockStore.getState().locked === locked) return;
  useLockStore.setState({ locked });
  if (locked) {
    const pending = useConfirmStore.getState().pending;
    useConfirmStore.setState({ pending: null });
    pending?.resolve(false);
  }
}

// ───────────────────────── Menu ─────────────────────────

type MenuItem = {
  kind?: "item";
  label: string;
  icon?: string;
  checked?: boolean;
  hint?: string;
  danger?: boolean;
  disabled?: boolean;
  /** Choosing it leaves the menu open, as ticking a box in a list does. */
  keepOpen?: boolean;
  onSelect: () => void;
};

/**
 * A row of options under a label, like a segmented control: picking one keeps the menu open. With the
 * keyboard, ←/→ pick the previous or next option while the row has the focus.
 */
export type MenuChoice = {
  kind: "choice";
  label: string;
  options: { value: string; label: string }[];
  value: string;
  onChange: (value: string) => void;
};

export type MenuEntry = MenuItem | MenuChoice | { kind: "separator" } | { kind: "header"; label: string };

const isItem = (e: MenuEntry): e is MenuItem => e.kind === undefined || e.kind === "item";

export interface MenuAnchor {
  x: number;
  y: number;
  /** Align the menu's right edge to x instead of its left edge. */
  alignRight?: boolean;
}

/** Floating menu (the design's language picker look). Closes on outside click / Escape. */
export function Menu({
  anchor,
  entries,
  onClose,
  minWidth,
}: {
  anchor: MenuAnchor;
  entries: MenuEntry[];
  onClose: () => void;
  minWidth?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: anchor.x, top: anchor.y });
  const [focus, setFocus] = useState(-1);
  const items = entries.flatMap((e, i) => (e.kind === "separator" || e.kind === "header" ? [] : [i]));
  const hasCheck = entries.some((x) => isItem(x) && x.checked !== undefined);
  const locked = useOverlaysLocked();

  useEffect(() => {
    if (locked) onClose();
  }, [locked, onClose]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    let left = anchor.alignRight ? anchor.x - r.width : anchor.x;
    let top = anchor.y;
    left = Math.max(8, Math.min(left, window.innerWidth - r.width - 8));
    if (top + r.height > window.innerHeight - 8) top = Math.max(8, anchor.y - r.height - 8);
    setPos({ left, top });
  }, [anchor.x, anchor.y, anchor.alignRight]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // Only the menu closes: not the sheet it opened in, which would drop the form's edits.
        e.preventDefault();
        e.stopPropagation();
        onClose();
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setFocus((f) => {
          const idx = items.indexOf(f);
          const next = e.key === "ArrowDown" ? idx + 1 : idx - 1;
          return items[(next + items.length) % items.length] ?? -1;
        });
      } else if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && entries[focus]?.kind === "choice") {
        e.preventDefault();
        const choice = entries[focus] as MenuChoice;
        const at = choice.options.findIndex((o) => o.value === choice.value);
        const next = choice.options[at + (e.key === "ArrowRight" ? 1 : -1)];
        if (next) choice.onChange(next.value);
      } else if (e.key === "Enter" && focus >= 0) {
        e.preventDefault();
        const entry = entries[focus];
        if (entry?.kind === "choice") onClose();
        else if (entry && isItem(entry) && !entry.disabled) {
          if (!entry.keepOpen) onClose();
          entry.onSelect();
        }
      }
    };
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", onClose);
    };
  }, [entries, focus, items, onClose]);

  if (locked) return null;
  return createPortal(
    <div ref={ref} role="menu" className={o.menu} style={{ ...pos, minWidth }}>
      {entries.map((e, i) => {
        if (e.kind === "separator") return <div key={i} className={o.menuSeparator} />;
        if (e.kind === "header")
          return (
            <div key={i} className={o.menuHeader}>
              {e.label}
            </div>
          );
        if (e.kind === "choice")
          return (
            <div
              key={i}
              role="group"
              aria-label={e.label}
              className={cx(o.menuChoice, focus === i && o.menuChoiceFocused)}
              onMouseEnter={() => setFocus(i)}
            >
              <div className={o.menuChoiceLabel}>{e.label}</div>
              <div className={o.menuChoiceOptions}>
                {e.options.map((opt) => (
                  <button
                    key={opt.value}
                    type="button"
                    role="menuitemradio"
                    aria-checked={opt.value === e.value}
                    className={cx(o.menuChoiceOption, opt.value === e.value && o.menuChoiceOn)}
                    onClick={() => e.onChange(opt.value)}
                  >
                    {opt.label}
                  </button>
                ))}
              </div>
            </div>
          );
        return (
          <button
            key={i}
            type="button"
            role={e.checked === undefined ? "menuitem" : e.keepOpen ? "menuitemcheckbox" : "menuitemradio"}
            aria-checked={e.checked}
            disabled={e.disabled}
            className={cx(o.menuItem, focus === i && o.menuItemFocused, e.danger && o.menuItemDanger)}
            onMouseEnter={() => setFocus(i)}
            onClick={() => {
              if (!e.keepOpen) onClose();
              e.onSelect();
            }}
          >
            {hasCheck && <span className={o.menuCheck}>{e.checked && <Icon name="check" />}</span>}
            {e.icon && <Icon name={e.icon} className={o.menuIcon} />}
            <span className={o.menuLabel}>{e.label}</span>
            {e.hint && <span className={o.menuHint}>{e.hint}</span>}
          </button>
        );
      })}
    </div>,
    document.body,
  );
}

/** State helper for menus anchored to a button or a right-click position. */
export function useMenu() {
  const [anchor, setAnchor] = useState<MenuAnchor | null>(null);
  const openAt = useCallback((a: MenuAnchor) => setAnchor(a), []);
  const openBelow = useCallback((el: HTMLElement, alignRight = false) => {
    const r = el.getBoundingClientRect();
    setAnchor({ x: alignRight ? r.right : r.left, y: r.bottom + 4, alignRight });
  }, []);
  const close = useCallback(() => setAnchor(null), []);
  return { anchor, openAt, openBelow, close };
}

// ───────────────────────── PopupSelect ─────────────────────────

export interface SelectOption<V> {
  value: V;
  label: string;
  /** Secondary monospace text, e.g. key type or `user@host:port`. */
  hint?: string;
  icon?: string;
}

/** The design's popup button (label + caret-up-down) with a menu of options. */
export function PopupSelect<V>({
  value,
  options,
  onChange,
  icon,
  placeholder,
  minWidth,
  disabled,
  ariaLabel,
  showHint = true,
}: {
  value: V;
  options: SelectOption<V>[];
  onChange: (v: V) => void;
  icon?: string;
  placeholder?: string;
  minWidth?: number;
  disabled?: boolean;
  ariaLabel?: string;
  showHint?: boolean;
}) {
  const menu = useMenu();
  const ref = useRef<HTMLButtonElement>(null);
  const selected = options.find((opt) => Object.is(opt.value, value));
  return (
    <>
      <button
        ref={ref}
        type="button"
        aria-label={ariaLabel}
        aria-haspopup="menu"
        disabled={disabled}
        className={controlStyles.popup}
        style={{ minWidth }}
        onClick={() => ref.current && menu.openBelow(ref.current)}
      >
        {(selected?.icon ?? icon) && <Icon name={selected?.icon ?? icon!} className={controlStyles.popupIcon} />}
        <span className={controlStyles.popupLabel} style={selected ? undefined : { color: "var(--fg2)" }}>
          {selected?.label ?? placeholder}
        </span>
        {showHint && selected?.hint && <span className={controlStyles.popupHint}>{selected.hint}</span>}
        <Icon name="caret-up-down" className={controlStyles.popupCaret} />
      </button>
      {menu.anchor && (
        <Menu
          anchor={menu.anchor}
          onClose={menu.close}
          minWidth={ref.current?.offsetWidth}
          entries={options.map((opt) => ({
            label: opt.label,
            hint: opt.hint,
            checked: Object.is(opt.value, value),
            onSelect: () => onChange(opt.value),
          }))}
        />
      )}
    </>
  );
}

// ───────────────────────── Modal / Sheet / Dialog ─────────────────────────

/** Backdrop + focus trap-lite + Escape handling. */
export function Modal({
  children,
  onClose,
  center,
  closeOnBackdrop = true,
}: {
  children: ReactNode;
  onClose?: () => void;
  center?: boolean;
  closeOnBackdrop?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const locked = useOverlaysLocked();
  // Captured while rendering: before the effect runs, React has already focused a control with
  // autoFocus inside the dialog.
  const [opener] = useState(() => document.activeElement as HTMLElement | null);
  // Callers often pass a new function on every render. Reading it through a ref keeps the effect
  // from running again and pulling focus back to the first control.
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useEffect(() => {
    if (locked) return;
    const node = ref.current;
    if (!node?.contains(document.activeElement)) {
      // Skips controls taken out of the tab order, such as the unselected tabs of a tablist.
      const first = node?.querySelector<HTMLElement>(
        ["input:not([disabled])", "textarea", "button:not([disabled])"]
          .map((sel) => `${sel}:not([tabindex="-1"])`)
          .join(", "),
      );
      first?.focus();
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && closeRef.current) {
        e.stopPropagation();
        closeRef.current();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
      // Development mode also runs this while the dialog stays open, to run the effect twice.
      if (!node?.isConnected && !useLockStore.getState().locked) opener?.focus?.();
    };
  }, [opener, locked]);
  // Kept mounted, so the dialog comes back as it was after unlocking.
  if (locked) return null;
  return createPortal(
    <div
      ref={ref}
      className={cx(o.backdrop, center && o.backdropCenter)}
      onMouseDown={(e) => {
        if (closeOnBackdrop && e.target === e.currentTarget) onClose?.();
      }}
    >
      {children}
    </div>,
    document.body,
  );
}

/** Wizard-style sheet: 540px card with a footer of actions (design §05). */
export function Sheet({
  children,
  footer,
  width,
  onClose,
  closeOnBackdrop,
}: {
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
  onClose?: () => void;
  closeOnBackdrop?: boolean;
}) {
  return (
    <Modal onClose={onClose} closeOnBackdrop={closeOnBackdrop}>
      <div role="dialog" aria-modal className={o.sheet} style={{ width }}>
        <div className={o.sheetBody}>{children}</div>
        {footer && <div className={o.sheetFooter}>{footer}</div>}
      </div>
    </Modal>
  );
}

export function SheetHeader({ title, subtitle }: { title: ReactNode; subtitle?: ReactNode }) {
  return (
    <div>
      <div className={o.sheetTitle}>{title}</div>
      {subtitle && <div className={o.sheetSubtitle}>{subtitle}</div>}
    </div>
  );
}

export const FooterSpacer = () => <div className={o.sheetFooterSpacer} />;

export interface ConfirmOptions {
  title: string;
  body?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  danger?: boolean;
  icon?: string;
  iconColor?: string;
}

/** Alert-style dialog. */
export function Dialog({
  title,
  body,
  icon,
  iconColor,
  actions,
  onClose,
  children,
}: {
  title: string;
  body?: ReactNode;
  icon?: string;
  iconColor?: string;
  actions: ReactNode;
  onClose?: () => void;
  children?: ReactNode;
}) {
  return (
    <Modal onClose={onClose} center>
      <div role="alertdialog" aria-modal className={o.dialog}>
        <div className={o.dialogHead}>
          {icon && <Icon name={icon} size={20} color={iconColor ?? "var(--accent)"} />}
          <div className={o.dialogTitle}>{title}</div>
        </div>
        {body && <div className={o.dialogBody}>{body}</div>}
        {children}
        <div className={o.dialogActions}>{actions}</div>
      </div>
    </Modal>
  );
}

// Imperative confirm(): `if (await confirm({...})) ...`
const useConfirmStore = create<{
  pending: (ConfirmOptions & { resolve: (ok: boolean) => void }) | null;
}>(() => ({ pending: null }));

export function confirm(options: ConfirmOptions): Promise<boolean> {
  if (useLockStore.getState().locked) return Promise.resolve(false);
  return new Promise((resolve) => useConfirmStore.setState({ pending: { ...options, resolve } }));
}

export function ConfirmHost() {
  const t = useT();
  const pending = useConfirmStore((st) => st.pending);
  if (!pending) return null;
  const done = (ok: boolean) => {
    useConfirmStore.setState({ pending: null });
    pending.resolve(ok);
  };
  return (
    <Dialog
      title={pending.title}
      body={pending.body}
      icon={pending.icon ?? (pending.danger ? "warning-circle" : undefined)}
      iconColor={pending.iconColor ?? (pending.danger ? "var(--red)" : undefined)}
      onClose={() => done(false)}
      actions={
        <>
          <Button onClick={() => done(false)}>{pending.cancelLabel ?? t("btn.cancel")}</Button>
          <Button variant={pending.danger ? "danger" : "primary"} onClick={() => done(true)} autoFocus>
            {pending.confirmLabel}
          </Button>
        </>
      }
    />
  );
}

// ───────────────────────── Toasts ─────────────────────────

interface Toast {
  id: number;
  text: string;
  tone: "info" | "success" | "error";
}

const useToastStore = create<{ toasts: Toast[] }>(() => ({ toasts: [] }));
let toastSeq = 0;

export function toast(text: string, tone: Toast["tone"] = "info", ms = 2600) {
  const id = ++toastSeq;
  useToastStore.setState((st) => ({ toasts: [...st.toasts, { id, text, tone }] }));
  setTimeout(() => useToastStore.setState((st) => ({ toasts: st.toasts.filter((x) => x.id !== id) })), ms);
}

export function ToastHost() {
  const toasts = useToastStore((st) => st.toasts);
  return createPortal(
    <div className={o.toasts} role="status" aria-live="polite">
      {toasts.map((x) => (
        <div key={x.id} className={cx(o.toast, x.tone === "error" && o.toastError, x.tone === "success" && o.toastSuccess)}>
          <Icon
            name={x.tone === "error" ? "warning-circle" : x.tone === "success" ? "check-circle" : "info"}
            fill={x.tone !== "info"}
            size={15}
          />
          {x.text}
        </div>
      ))}
    </div>,
    document.body,
  );
}
