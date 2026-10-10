import { useSyncExternalStore } from "react";
import { messages, type Locale, type MessageKey } from "./locales";

export type { Locale, MessageKey };
export const LOCALES: Locale[] = ["zh-CN", "en", "ja"];

/** Picks a supported locale from the OS / WebView language list (default: system language). */
export function detectLocale(languages: readonly string[] = navigator.languages ?? [navigator.language]): Locale {
  for (const raw of languages) {
    const lang = raw.toLowerCase();
    if (lang.startsWith("zh")) return "zh-CN";
    if (lang.startsWith("ja")) return "ja";
    if (lang.startsWith("en")) return "en";
  }
  return "en";
}

let current: Locale = detectLocale();
const listeners = new Set<() => void>();

export function getLocale(): Locale {
  return current;
}

export function setLocale(locale: Locale) {
  if (locale === current) return;
  current = locale;
  document.documentElement.lang = locale;
  listeners.forEach((l) => l());
}

export type Params = Record<string, string | number>;

/**
 * Translate `key`. `{name}` placeholders are replaced from `params`. When `params.n` is a number
 * and a `key_one` variant exists, it is used for n === 1 (English plurals).
 */
export function translate(locale: Locale, key: MessageKey, params?: Params): string {
  const table = messages[locale] as Record<string, string>;
  let text: string | undefined;
  if (params && typeof params.n === "number" && params.n === 1) text = table[`${key}_one`];
  text ??= table[key] ?? (messages.en as Record<string, string>)[key] ?? key;
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (m, name: string) => (name in params ? String(params[name]) : m));
}

export function t(key: MessageKey, params?: Params): string {
  return translate(current, key, params);
}

function subscribe(cb: () => void) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** React hook: re-renders on language change (switching takes effect immediately, design §07). */
export function useT() {
  const locale = useSyncExternalStore(subscribe, getLocale);
  return Object.assign((key: MessageKey, params?: Params) => translate(locale, key, params), { locale });
}

const dateFormatters = new Map<string, Intl.DateTimeFormat>();
function fmt(locale: Locale, opts: Intl.DateTimeFormatOptions) {
  // A formatter keeps the time zone it was made in, and the system's can change while the app runs.
  // The UTC offset is a cheap stand-in for the zone (a DST change only adds formatters).
  const k = `${locale}${new Date().getTimezoneOffset()}${JSON.stringify(opts)}`;
  let f = dateFormatters.get(k);
  if (!f) dateFormatters.set(k, (f = new Intl.DateTimeFormat(locale, opts)));
  return f;
}

/** Local midnight of the day `ms` falls on, or of a day `days` from it. A day is not always 24 hours (DST). */
export function startOfDay(ms: number, days = 0): number {
  const d = new Date(ms);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + days).getTime();
}

const dayListeners = new Set<() => void>();
let dayTimer: number | undefined;

function subscribeDay(cb: () => void) {
  dayListeners.add(cb);
  // A check every minute rather than a timer to midnight, which sleep or a time zone change would throw off.
  dayTimer ??= window.setInterval(() => dayListeners.forEach((l) => l()), 60_000);
  return () => {
    dayListeners.delete(cb);
    if (dayListeners.size > 0) return;
    window.clearInterval(dayTimer);
    dayTimer = undefined;
  };
}

const todayStart = () => startOfDay(Date.now());

/** React hook: local midnight of today, which re-renders when the day changes, for labels that name the day. */
export function useToday(): number {
  return useSyncExternalStore(subscribeDay, todayStart);
}

/** "2 min ago", "Today 09:12", "Yesterday", "Oct 5" — the design's last-connected style. */
export function formatRelative(locale: Locale, ms: number, now = Date.now()): string {
  const diff = Math.max(0, now - ms);
  const min = Math.floor(diff / 60_000);
  if (min < 1) return translate(locale, "time.justNow");
  if (min < 60) return translate(locale, "time.minutesAgo", { n: min });
  const d = new Date(ms);
  const hm = fmt(locale, { hour: "2-digit", minute: "2-digit", hour12: false }).format(d);
  if (ms >= startOfDay(now)) return translate(locale, "time.todayAt", { time: hm });
  if (ms >= startOfDay(now, -1)) return translate(locale, "time.yesterdayAt", { time: hm });
  const sameYear = d.getFullYear() === new Date(now).getFullYear();
  return fmt(locale, sameYear ? { month: "short", day: "numeric" } : { year: "numeric", month: "short", day: "numeric" }).format(d);
}

/**
 * A message's time: "09:12" today, "Yesterday 09:12", "Oct 5, 09:12", and the year too before this
 * one. Only the day of `now` matters, so `useToday()` can stand in for it.
 */
export function formatMessageTime(locale: Locale, ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const clock = { hour: "2-digit", minute: "2-digit", hour12: false } as const;
  if (ms >= startOfDay(now)) return fmt(locale, clock).format(d);
  if (ms >= startOfDay(now, -1)) return translate(locale, "time.yesterdayAt", { time: fmt(locale, clock).format(d) });
  const year = d.getFullYear() === new Date(now).getFullYear() ? {} : ({ year: "numeric" } as const);
  return fmt(locale, { ...year, month: "short", day: "numeric", ...clock }).format(d);
}

/** The full date and time, for a tooltip. */
export function formatDateTime(locale: Locale, ms: number): string {
  return fmt(locale, { dateStyle: "medium", timeStyle: "medium" }).format(new Date(ms));
}

/** "2026-03-14" */
export function formatDate(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 10 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}
