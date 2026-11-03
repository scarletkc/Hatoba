import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function mount(text: string, details?: boolean): HTMLDivElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<Markdown text={text} details={details} />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host;
}

// As the release notes write it.
const notes =
  "## Fixes\n\n- one\n\n<details>\n<summary>Other changes</summary>\n\n- two\n- three\n\n</details>\n\nAfter.";

describe("Markdown <details>", () => {
  it("shows a closed section that its summary opens and closes", () => {
    const host = mount(notes, true);
    const button = host.querySelector("button[aria-expanded]") as HTMLButtonElement;
    expect(button.textContent).toBe("Other changes");
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(host.textContent).not.toContain("two");
    expect(host.textContent).not.toContain("<");
    expect(host.textContent).toContain("After.");

    act(() => button.click());
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect([...host.querySelectorAll("li")].map((li) => li.textContent)).toEqual(["one", "two", "three"]);

    act(() => button.click());
    expect(host.textContent).not.toContain("two");
  });

  it("reads the summary on one line with the tag, or as its own block, and starts open with `open`", () => {
    for (const text of [
      "<details open><summary>More</summary>\n\nInside.\n\n</details>",
      "<details open>\n\n<summary>More</summary>\n\nInside.\n\n</details>",
    ]) {
      const host = mount(text, true);
      expect(host.querySelector("button")?.textContent, text).toBe("More");
      expect(host.textContent, text).toBe("MoreInside.");
      act(() => unmount?.());
    }
  });

  it("keeps the tags as text without a summary, in messages, and for any other HTML", () => {
    expect(mount("<details>\n\nInside.\n\n</details>", true).textContent).toBe("<details>\nInside.\n</details>");
    act(() => unmount?.());
    expect(mount(notes).querySelector("button")).toBeNull();
    act(() => unmount?.());
    expect(mount("<b>bold</b>", true).textContent).toBe("<b>bold</b>");
  });
});
