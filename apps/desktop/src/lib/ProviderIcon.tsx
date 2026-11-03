import { Icon } from "@hatoba/ui";
import { cx } from "./cx";
import s from "./ProviderIcon.module.css";

/** The brands whose logo shows for an AI provider; `openai` is Phosphor's, the others are in `providers/`. */
export type ProviderBrand =
  | "anthropic"
  | "openai"
  | "ollama"
  | "lmstudio"
  | "openrouter"
  | "deepseek"
  | "googlegemini"
  | "mistralai"
  | "moonshotai"
  | "kimi"
  | "zdotai"
  | "alibabacloud"
  | "perplexity"
  | "nvidia"
  | "cloudflare"
  | "github"
  | "baidu"
  | "vercel"
  | "poe";

/** Each brand's API domains: the domain itself or any subdomain of it. */
const DOMAINS: [string, ProviderBrand][] = [
  ["anthropic.com", "anthropic"],
  ["openai.com", "openai"],
  ["chatgpt.com", "openai"],
  ["ollama.com", "ollama"],
  ["openrouter.ai", "openrouter"],
  ["deepseek.com", "deepseek"],
  ["generativelanguage.googleapis.com", "googlegemini"],
  ["mistral.ai", "mistralai"],
  ["moonshot.ai", "moonshotai"],
  ["moonshot.cn", "moonshotai"],
  ["kimi.com", "kimi"],
  ["z.ai", "zdotai"],
  ["perplexity.ai", "perplexity"],
  ["nvidia.com", "nvidia"],
  ["cloudflare.com", "cloudflare"],
  ["github.ai", "github"],
  ["baidubce.com", "baidu"],
  ["vercel.sh", "vercel"],
  ["poe.com", "poe"],
];

/** The default ports of local servers, which a base URL on any host keeps. */
const PORTS: Record<string, ProviderBrand> = { "11434": "ollama", "1234": "lmstudio" };

/**
 * The brand of a provider, from what its record holds: a ChatGPT sign-in is OpenAI, else the base
 * URL's domain, or the default port of Ollama or LM Studio. Null for anything else, such as a relay
 * or a server that could be serving any model.
 */
export function providerBrand(p: { base_url: string; chatgpt?: unknown }): ProviderBrand | null {
  if (p.chatgpt) return "openai";
  let url: URL;
  try {
    url = new URL(p.base_url.trim());
  } catch {
    return null;
  }
  const host = url.hostname.toLowerCase().replace(/\.$/, "");
  for (const [domain, brand] of DOMAINS) if (host === domain || host.endsWith(`.${domain}`)) return brand;
  if (/^dashscope[a-z0-9-]*\.aliyuncs\.com$/.test(host)) return "alibabacloud";
  return PORTS[url.port] ?? null;
}

/**
 * AI-01: the logo of a provider's brand, painted in its color as the hosts list paints OS logos
 * (HOST-11), or the generic sparkle when the brand is unknown.
 */
export function ProviderIcon({
  provider,
  className,
}: {
  provider: { base_url: string; chatgpt?: unknown };
  className?: string;
}) {
  const brand = providerBrand(provider);
  if (brand === "openai") return <Icon name="open-ai-logo" fill className={cx(s.glyph, s.openai, className)} />;
  if (brand) return <span className={cx(s.logo, s[brand], className)} />;
  return <Icon name="sparkle" className={cx(s.glyph, s.generic, className)} />;
}
