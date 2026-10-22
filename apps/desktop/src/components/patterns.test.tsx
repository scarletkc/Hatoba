import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Chip, ErrorText, HintText, ListRow, StatusLine } from "./patterns";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});

function render(ui: React.ReactNode): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(ui));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return host;
}

describe("ListRow", () => {
  it("puts the status under the title and leaves out empty slots", () => {
    const host = render(
      <ListRow status={<span data-testid="status">ok</span>}>
        <span data-testid="title">rule</span>
      </ListRow>,
    );
    const row = host.firstElementChild!;
    expect(row.children).toHaveLength(1);
    const body = row.firstElementChild!;
    expect([...body.children].map((c) => c.getAttribute("data-testid"))).toEqual(["title", "status"]);
  });

  it("renders leading and trailing around the body", () => {
    const host = render(
      <ListRow leading={<i data-testid="lead" />} trailing={<button type="button">Start</button>}>
        rule
      </ListRow>,
    );
    const row = host.firstElementChild!;
    expect(row.children).toHaveLength(3);
    expect(row.querySelector("[data-testid=lead]")).not.toBeNull();
    expect(row.lastElementChild!.textContent).toBe("Start");
  });
});

describe("StatusLine", () => {
  it("announces progress and shows a spinner", () => {
    const host = render(<StatusLine tone="progress">Starting…</StatusLine>);
    const line = host.firstElementChild!;
    expect(line.getAttribute("role")).toBe("status");
    expect(line.querySelector(".ph-circle-notch")).not.toBeNull();
  });

  it("gives each tone its icon", () => {
    const host = render(
      <>
        <StatusLine tone="success">Running</StatusLine>
        <StatusLine tone="error">Refused</StatusLine>
        <StatusLine>Idle</StatusLine>
      </>,
    );
    const [ok, failed, idle] = [...host.children];
    expect(ok.querySelector(".ph-check-circle")).not.toBeNull();
    expect(failed.querySelector(".ph-warning-circle")).not.toBeNull();
    expect(idle.querySelector("i")).toBeNull();
  });
});

describe("field messages", () => {
  it("announces an error but not a hint", () => {
    const host = render(
      <>
        <HintText>Up to 64 characters</HintText>
        <ErrorText>Too long</ErrorText>
      </>,
    );
    const [hint, error] = [...host.children];
    expect(hint.getAttribute("role")).toBeNull();
    expect(error.getAttribute("role")).toBe("alert");
  });
});

describe("Chip", () => {
  it("shows a spinner in place of its icon while busy", () => {
    const host = render(
      <Chip icon="check" busy>
        Running
      </Chip>,
    );
    expect(host.querySelector(".ph-circle-notch")).not.toBeNull();
    expect(host.querySelector(".ph-check")).toBeNull();
  });
});
