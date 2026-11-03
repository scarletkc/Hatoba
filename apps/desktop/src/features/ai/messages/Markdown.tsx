import { memo, useRef, useState, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { DisclosureCaret, Icon, controlStyles } from "@hatoba/ui";
import { useT } from "@/i18n";
import { openExternal, writeClipboard } from "@/lib/clipboard";
import { cx } from "@/lib/cx";
import { grammarOf, useHighlight, type Token } from "./highlight";
import s from "./Markdown.module.css";

interface MdNode {
  type: string;
  value?: string;
  data?: { hName?: string; hProperties?: Record<string, string | boolean> };
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

const DETAILS = /^<details(\s+open)?\s*>(?:\s*<summary>([^<]*)<\/summary>)?$/i;
const SUMMARY = /^<summary>([^<]*)<\/summary>$/i;
const DETAILS_END = /^<\/details>$/i;

/**
 * A `<details>` block with a `<summary>` becomes a section that opens and closes, as release notes
 * use it; the tags are read as markup and nothing else is, so every other tag stays text.
 */
function detailsBlocks() {
  const group = (nodes: MdNode[]): MdNode[] => {
    const out: MdNode[] = [];
    const open: MdNode[] = [];
    const into = () => open.at(-1)?.children ?? out;
    for (let i = 0; i < nodes.length; i++) {
      const node = nodes[i];
      const html = node.type === "html" ? (node.value ?? "").trim() : null;
      const start = html?.match(DETAILS);
      // The summary may follow the tag as a block of its own.
      const own = start && start[2] === undefined ? nodes[i + 1] : undefined;
      const summary = start?.[2] ?? (own?.type === "html" ? own.value?.trim().match(SUMMARY)?.[1] : undefined);
      if (start && summary !== undefined) {
        if (own) i++;
        const details: MdNode = {
          type: "details",
          data: { hName: "details", hProperties: { summary: summary.trim(), open: !!start[1] } },
          children: [],
        };
        into().push(details);
        open.push(details);
      } else if (html && DETAILS_END.test(html) && open.length > 0) {
        open.pop();
      } else {
        into().push(node);
      }
    }
    return out;
  };
  return (tree: MdNode) => {
    tree.children = group(tree.children ?? []);
  };
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
      <Icon name="image" size={14} />
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

/** A fenced block's `<code>` element, from the `pre` that react-markdown passes. */
function codeOf(pre: MdElement | undefined): { className: unknown; text: string } | null {
  const code = pre?.children?.find((c) => c.type === "element" && c.tagName === "code");
  if (!code) return null;
  const text = (code.children ?? []).map((c) => (c.type === "text" ? (c.value ?? "") : "")).join("");
  return { className: code.properties?.className, text };
}

interface MdElement {
  type: string;
  tagName?: string;
  value?: string;
  properties?: { className?: unknown };
  children?: MdElement[];
}

const classNames = (value: unknown) =>
  Array.isArray(value) ? value.join(" ") : typeof value === "string" ? value : undefined;

function tokens(nodes: readonly Token[]): ReactNode[] {
  return nodes.map((n, i) =>
    n.type === "text" ? (
      n.value
    ) : n.type === "element" ? (
      <span key={i} className={classNames(n.properties?.className)}>
        {tokens(n.children ?? [])}
      </span>
    ) : null,
  );
}

/** A fenced block, colored when its language is one `highlight.ts` knows, with a copy button on hover. */
function CodeBlock({ node, children }: { node?: MdElement; children?: ReactNode }) {
  const t = useT();
  const code = codeOf(node);
  const highlighted = useHighlight(code ? grammarOf(code.className) : null, code?.text ?? "");
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
        {highlighted ? (
          <code className={classNames(code?.className)}>
            {tokens(highlighted.tokens)}
            {highlighted.rest}
          </code>
        ) : (
          children
        )}
      </pre>
      <button
        type="button"
        className={s.copy}
        title={t(copied ? "btn.copied" : "btn.copy")}
        aria-label={t("btn.copy")}
        onClick={() => void copy()}
      >
        <Icon name={copied ? "check" : "copy"} size={14} />
      </button>
    </div>
  );
}

/** A `<details>` block, whose summary opens and closes it like the reasoning above a reply (AI-06). */
function Details({ summary, initial, children }: { summary: string; initial: boolean; children?: ReactNode }) {
  const [open, setOpen] = useState(initial);
  return (
    <div className={s.details}>
      <button type="button" className={s.summary} aria-expanded={open} onClick={() => setOpen((v) => !v)}>
        {summary}
        <DisclosureCaret open={open} size={12} />
      </button>
      {open && <div className={cx(s.detailsBody, controlStyles.reveal)}>{children}</div>}
    </div>
  );
}

const components: Components = {
  a: ({ href, children }) => <Link href={href}>{children}</Link>,
  img: ({ src, alt }) => <ImageLink src={src} alt={alt} />,
  pre: ({ node, children }) => <CodeBlock node={node}>{children}</CodeBlock>,
  table: ({ children }) => (
    <div className={s.tableWrap}>
      <table>{children}</table>
    </div>
  ),
  details: ({ node, children }) => (
    <Details summary={String(node?.properties.summary ?? "")} initial={node?.properties.open === true}>
      {children}
    </Details>
  ),
};

const plugins = [remarkGfm, rawHtmlAsText];
const pluginsWithDetails = [remarkGfm, detailsBlocks, rawHtmlAsText];

/** A message's Markdown (GFM), with a caret while it streams. `details` reads `<details>` blocks as markup. */
export const Markdown = memo(function Markdown({
  text,
  streaming,
  details,
}: {
  text: string;
  streaming?: boolean;
  details?: boolean;
}) {
  return (
    <div className={cx(s.md, streaming && s.streaming, "selectable")}>
      <ReactMarkdown remarkPlugins={details ? pluginsWithDetails : plugins} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
