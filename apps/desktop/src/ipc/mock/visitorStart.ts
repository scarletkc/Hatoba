import { useTabs } from "@/app/tabs";
import { suggestQuestion } from "@/features/ai/actions";
import { getLocale, type Locale } from "@/i18n";
import { VISITOR_HOST } from "./visitor";

/** The question the input starts with: the nginx 502 that `showcase` fixes, as the screenshots ask it. */
const QUESTION: Record<Locale, string> = {
  en: "Users are getting 502s from nginx on this box. Find out why and fix it.",
  "zh-CN": "这台机器上的 nginx 一直返回 502，找出原因并修好它。",
  ja: "nginx が 502 を返しています。原因を調べて直してください。",
};

/** What the secret fields hold, so nobody types a real password or token into a page that only pretends to use it. */
const SAMPLE_SECRET = "sample-secret-for-the-demo-7Qx2";

/** Starts the demo once the app is up: the question waits in the first tab, and secret fields fill themselves. */
export function startVisitor() {
  // The tab may have connected before this module loaded, so the state as it is counts too.
  const suggest = (st: ReturnType<typeof useTabs.getState>) => {
    const tab = st.tabs.find((x) => x.title === VISITOR_HOST && x.status === "connected");
    if (!tab) return false;
    suggestQuestion(tab.id, QUESTION[getLocale()]);
    return true;
  };
  if (!suggest(useTabs.getState())) {
    const stop = useTabs.subscribe((st) => {
      if (suggest(st)) stop();
    });
  }
  fillSecrets();
}

function fillSecrets() {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!;
  const filled = new WeakSet<HTMLInputElement>();
  const fill = (input: HTMLInputElement) => {
    if (filled.has(input) || input.value || input.disabled || input.readOnly) return;
    // Once per field, so one the visitor cleared stays clear.
    filled.add(input);
    setValue.call(input, SAMPLE_SECRET);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  };
  const scan = () => document.querySelectorAll<HTMLInputElement>("input[type=password]").forEach(fill);
  new MutationObserver(scan).observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["disabled", "readonly"] });
}
