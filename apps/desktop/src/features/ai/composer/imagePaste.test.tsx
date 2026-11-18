import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/ipc/api", async (importActual) => ({ ...(await importActual<typeof import("@/ipc/api")>()), api: {} }));
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { Composer } = await import("./Composer");
const { AttachmentCardFor } = await import("./AttachmentChips");
const { blankSlot, getSlot, HOME_SLOT, useAi } = await import("../store");
const { imageBase64 } = await import("../images");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
});
beforeEach(() => {
  useAi.setState({ slots: { [HOME_SLOT]: blankSlot("manual") } });
});

function Harness() {
  const slot = useAi((state) => state.slots[HOME_SLOT]);
  return <Composer slotId={HOME_SLOT} slot={slot} model={{ provider_id: "p1", model_id: "m1" }} tools={false} />;
}

function render() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<Harness />));
  unmount = () => {
    root.unmount();
    host.remove();
  };
  return { host, root };
}

describe("pasting images into the AI input", () => {
  it.each(["image/png", ""])(
    "attaches clipboard images (%s), keeps accompanying text and supports removal",
    async (mime) => {
      const { host } = render();
      const input = host.querySelector("textarea")!;
      const bytes = new Uint8Array([137, 80, 78, 71, 13, 10, 26, 10]);
      const file = new File([bytes], "screenshot.png", { type: mime });
      Object.defineProperty(file, "arrayBuffer", { value: async () => bytes.buffer });
      const paste = new Event("paste", { bubbles: true, cancelable: true });
      Object.defineProperty(paste, "clipboardData", { value: { files: [file], getData: () => "Explain this error" } });
      await act(async () => {
        input.dispatchEvent(paste);
      });
      expect(paste.defaultPrevented).toBe(true);
      expect(getSlot(HOME_SLOT).draft).toBe("Explain this error");
      expect(getSlot(HOME_SLOT).extras[0].attachment).toEqual({
        kind: "image",
        name: "screenshot.png",
        mime: "image/png",
        text: imageBase64(bytes),
      });
      const chip = [...host.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")].find((button) =>
        button.textContent?.includes("screenshot.png"),
      )!;
      act(() => chip.click());
      expect(host.querySelector("img")?.getAttribute("src")).toBe(`data:image/png;base64,${imageBase64(bytes)}`);
      const remove = [...host.querySelectorAll<HTMLButtonElement>("button[aria-label]")].find((button) =>
        button.getAttribute("aria-label")?.includes("screenshot.png"),
      )!;
      act(() => remove.click());
      expect(getSlot(HOME_SLOT).extras).toEqual([]);
    },
  );

  it("renders sent images without displaying the base64 body", () => {
    const { host, root } = render();
    act(() =>
      root.render(
        <AttachmentCardFor attachment={{ kind: "image", name: "screen.png", mime: "image/png", text: "aGk=" }} />,
      ),
    );
    expect(host.querySelector("img")?.getAttribute("src")).toBe("data:image/png;base64,aGk=");
    expect(host.textContent).not.toContain("aGk=");
  });
});
