import { describe, expect, it } from "vitest";
import { frameCodeBlocks } from "./markdown";

describe("frameCodeBlocks", () => {
  it("frames each code block and leaves the rest", () => {
    const html = '<p>Run <code>ls</code>:</p><pre class="astro-code"><code>ls\n</code></pre><pre><code>pwd</code></pre><p>preface</p>';
    expect(frameCodeBlocks(html)).toBe(
      '<p>Run <code>ls</code>:</p><div class="code-block" data-copy-scope><pre class="astro-code"><code>ls\n</code></pre></div>' +
        '<div class="code-block" data-copy-scope><pre><code>pwd</code></pre></div><p>preface</p>',
    );
  });
});
