import { act, useRef } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { useFollowBottom, type FollowBottom } from "./followBottom";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// jsdom lays nothing out: an element's content and visible heights come from `data-content` and
// `data-view`, and its scroll position is clamped to them as a browser does.
const tops = new WeakMap<HTMLElement, number>();
const maxTop = (el: HTMLElement) => Math.max(0, el.scrollHeight - el.clientHeight);
beforeAll(() => {
  Object.defineProperties(HTMLElement.prototype, {
    scrollHeight: {
      configurable: true,
      get(this: HTMLElement) {
        return Number(this.dataset.content ?? 0);
      },
    },
    clientHeight: {
      configurable: true,
      get(this: HTMLElement) {
        return Number(this.dataset.view ?? 0);
      },
    },
    scrollTop: {
      configurable: true,
      get(this: HTMLElement) {
        return Math.min(tops.get(this) ?? 0, maxTop(this));
      },
      set(this: HTMLElement, v: number) {
        tops.set(this, Math.max(0, Math.min(v, maxTop(this))));
      },
    },
  });
});
afterAll(() => {
  for (const key of ["scrollHeight", "clientHeight", "scrollTop"]) delete (HTMLElement.prototype as unknown as Record<string, unknown>)[key];
});

// The observer the hook makes, to report a size change as the browser would.
let resized: ((entries: { target: Element }[]) => void) | null = null;
vi.stubGlobal(
  "ResizeObserver",
  class {
    constructor(callback: (entries: { target: Element }[]) => void) {
      resized = callback;
    }
    observe() {}
    unobserve() {}
    disconnect() {}
  },
);

let bottom: FollowBottom;
function List({ content, live }: { content: number; live: boolean }) {
  const ref = useRef<HTMLDivElement>(null);
  bottom = useFollowBottom(ref, live);
  return (
    <div ref={ref} data-content={content} data-view={100} onScroll={bottom.onScroll} onWheel={bottom.onWheel} onKeyDown={bottom.onKeyDown}>
      <p>reply</p>
      <textarea />
      <div className="inner" style={{ overflowY: "auto" }} data-content={200} data-view={50} />
    </div>
  );
}

let root: Root | null = null;
let host: HTMLDivElement | null = null;
afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  root = null;
  host = null;
});

function mount(content: number, live = true) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root!.render(<List content={content} live={live} />));
  const list = host.firstElementChild as HTMLDivElement;
  return {
    list,
    render: (next: number, nextLive = live) => act(() => root!.render(<List content={next} live={nextLive} />)),
    scrollTo(top: number) {
      list.scrollTop = top;
      act(() => void list.dispatchEvent(new Event("scroll")));
    },
    fire: (target: Element, event: Event) => act(() => void target.dispatchEvent(event)),
  };
}

const wheelUp = () => new WheelEvent("wheel", { deltaY: -100, bubbles: true });
const key = (name: string) => new KeyboardEvent("keydown", { key: name, bubbles: true });

describe("following the bottom of the conversation", () => {
  it("follows new content while at the bottom", () => {
    const { list, render } = mount(300);
    expect(list.scrollTop).toBe(200);
    render(500);
    expect(list.scrollTop).toBe(400);
  });

  it("lets go on a wheel turn up before the next piece of the reply arrives", () => {
    const { list, render, fire } = mount(500);
    fire(list.querySelector("p")!, wheelUp());
    render(700);
    expect(list.scrollTop).toBe(400);
  });

  it("lets go when the list moves up a little", () => {
    const { list, render, scrollTo } = mount(500);
    scrollTo(390);
    render(700);
    expect(list.scrollTop).toBe(390);
  });

  it("lets go when the list moved up before its scroll event arrives", () => {
    const { list, render } = mount(500);
    list.scrollTop = 250;
    render(700);
    expect(list.scrollTop).toBe(250);
  });

  it("follows again only once a scroll down ends near the bottom", () => {
    const { list, render, scrollTo } = mount(500);
    scrollTo(250);
    scrollTo(300);
    render(600);
    expect(list.scrollTop).toBe(300);
    scrollTo(470);
    render(800);
    expect(list.scrollTop).toBe(700);
  });

  it("keeps following when content shrinks under it", () => {
    const { list, render } = mount(500);
    render(300);
    render(600);
    expect(list.scrollTop).toBe(500);
  });

  it("lets go on a key up, but not one typed in a text field", () => {
    const { list, render, fire } = mount(500);
    fire(list.querySelector("textarea")!, key("ArrowUp"));
    render(700);
    expect(list.scrollTop).toBe(600);

    fire(list.querySelector("p")!, key("PageUp"));
    render(900);
    expect(list.scrollTop).toBe(600);
  });

  it("leaves a wheel turn to a scroller inside the list that can still go up", () => {
    const { list, render, fire } = mount(500);
    const inner = list.querySelector<HTMLElement>(".inner")!;
    inner.scrollTop = 20;
    fire(inner, wheelUp());
    render(700);
    expect(list.scrollTop).toBe(600);
  });

  it("follows again from the bottom when the user sends a message", () => {
    const { list, render, scrollTo } = mount(500);
    scrollTo(100);
    act(() => bottom.stick());
    expect(list.scrollTop).toBe(400);
    render(700);
    expect(list.scrollTop).toBe(600);
  });

  it("follows content that grows on its own only while a turn runs, and the list's own size always", () => {
    const { list, render } = mount(500, true);
    list.dataset.content = "650";
    act(() => resized!([{ target: list.firstElementChild! }]));
    expect(list.scrollTop).toBe(550);

    render(650, false);
    list.dataset.content = "800";
    act(() => resized!([{ target: list.firstElementChild! }]));
    expect(list.scrollTop).toBe(550);
    act(() => resized!([{ target: list }]));
    expect(list.scrollTop).toBe(700);
  });
});
