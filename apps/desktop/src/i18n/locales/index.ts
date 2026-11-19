import ai from "./ai";
import aiSettings from "./aiSettings";
import common from "./common";
import dashboard from "./dashboard";
import hosts from "./hosts";
import keys from "./keys";
import settings from "./settings";
import sync from "./sync";
import terminal from "./terminal";
import vault from "./vault";

export const tables = [common, hosts, dashboard, terminal, keys, sync, vault, settings, ai, aiSettings] as const;

export type Locale = "zh-CN" | "zh-TW" | "en" | "ja";

type UnionToIntersection<U> = (U extends unknown ? (k: U) => void : never) extends (k: infer I) => void ? I : never;
type AllZh = UnionToIntersection<(typeof tables)[number]["zh-CN"]>;

export type MessageKey = keyof AllZh & string;

function merge(locale: Locale): Record<string, string> {
  return Object.assign({}, ...tables.map((t) => t[locale]));
}

export const messages: Record<Locale, Record<string, string>> = {
  "zh-CN": merge("zh-CN"),
  "zh-TW": merge("zh-TW"),
  en: merge("en"),
  ja: merge("ja"),
};
