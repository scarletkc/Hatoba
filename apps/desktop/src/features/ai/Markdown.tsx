import { memo, useRef, useState, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { Icon } from "@/components/controls";
import { openExternal, writeClipboard } from "@/features/terminal/clipboard";
import { useT } from "@/i18n";
import { cx } from "@/lib/cx";
import s from "./Markdown.module.css";

interface MdNode {
  type: string;
  children?: MdNode[];
}

/** Raw HTML in a message shows as the text it is: nothing is ever parsed as HTML (§9). */
function rawHtmlAsText() {
  const walk = (node: MdNode) => {
    if (node.type === "html") node.type = "text";
    node.children?.forEach(walk);
  };
  return (tree: MdNode) => walk(tree);
}

const isWebUrl = (url: string | undefined): url is string => !!url && /^https?:\/\//i.test(url);

/** Links open in the system browser (§9); anything else stays text. */
function Link({ href, children }: { href?: string; children?: ReactNode }) {
  if (!isWebUrl(href)) return <span className={s.badLink}>{children}</span>;
  return (
    <a
      href={href}
      title={href}
      onClick={(e) => {
        e.preventDefault();
        void openExternal(href);
      }}
    >
      {children}
    </a>
  );
}

/** No remote images (§9): an image becomes a link to it. */
function ImageLink({ src, alt }: { src?: unknown; alt?: string }) {
  const url = typeof src === "string" ? src : undefined;
  const label = alt || url || "";
  const body = (
    <>
      <Icon name="image" size={13} />
      {label}
    </>
  );
  return isWebUrl(url) ? (
    <a
      href={url}
      title={url}
      className={s.image}
      onClick={(e) => {
        e.preventDefault();
        void openExternal(url);
      }}
    >
      {body}
    </a>
  ) : (
    <span className={s.image}>{body}</span>
  );
}

function CodeBlock({ children }: { children?: ReactNode }) {
  const t = useT();
  const ref = useRef<HTMLPreElement>(null);
  const [copied, setCopied] = useState(false);
  const copy = async () => {
    try {
      await writeClipboard(ref.current?.innerText.replace(/\n$/, "") ?? "");
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      /* the clipboard is unavailable; nothing to do */
    }
  };
  return (
    <div className={s.codeBlock}>
      <pre ref={ref} className="selectable">
        {children}
      </pre>
      <button type="button" className={s.copy} title={t(copied ? "btn.copied" : "btn.copy")} aria-label={t("btn.copy")} onClick={() => void copy()}>
        <Icon name={copied ? "check" : "copy"} size={13} />
      </button>
    </div>
  );
}

const components: Components = {
  a: ({ href, children }) => <Link href={href}>{children}</Link>,
  img: ({ src, alt }) => <ImageLink src={src} alt={alt} />,
  pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
  table: ({ children }) => (
    <div className={s.tableWrap}>
      <table>{children}</table>
    </div>
  ),
};

const plugins = [remarkGfm, rawHtmlAsText];

/** A message's Markdown (GFM), with a caret while it streams. */
export const Markdown = memo(function Markdown({ text, streaming }: { text: string; streaming?: boolean }) {
  return (
    <div className={cx(s.md, streaming && s.streaming, "selectable")}>
      <ReactMarkdown remarkPlugins={plugins} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
