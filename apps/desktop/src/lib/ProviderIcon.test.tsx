import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { ProviderIcon, providerBrand } from "./ProviderIcon";

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

describe("providerBrand", () => {
  it.each([
    ["https://api.anthropic.com", "anthropic"],
    ["https://api.openai.com/v1", "openai"],
    ["https://openrouter.ai/api/v1", "openrouter"],
    ["https://api.deepseek.com/anthropic", "deepseek"],
    ["https://generativelanguage.googleapis.com/v1beta/openai", "googlegemini"],
    ["https://api.moonshot.cn/v1", "moonshotai"],
    ["https://api.z.ai/api/paas/v4", "zdotai"],
    ["https://dashscope-intl.aliyuncs.com/compatible-mode/v1", "alibabacloud"],
    ["https://api.cloudflare.com/client/v4/accounts/x/ai/run", "cloudflare"],
    ["http://localhost:11434/v1", "ollama"],
    ["http://gpu-box.lan:11434/v1", "ollama"],
    ["http://127.0.0.1:1234/v1", "lmstudio"],
  ])("knows %s", (base_url, brand) => {
    expect(providerBrand({ base_url })).toBe(brand);
  });

  it.each([
    "https://api.typesafe.ai/v1",
    "https://api.groq.com/openai/v1",
    "https://openai.example.com/v1",
    "https://notanthropic.com",
    "https://biz.ai/v1",
    "http://localhost:8000/v1",
    "not a url",
  ])("knows no brand for %s", (base_url) => {
    expect(providerBrand({ base_url })).toBeNull();
  });

  it("takes a ChatGPT sign-in for OpenAI, whatever the base URL", () => {
    expect(providerBrand({ base_url: "https://chatgpt.example/backend", chatgpt: { status: "signed_in" } })).toBe(
      "openai",
    );
  });
});

describe("ProviderIcon", () => {
  it("draws OpenAI with the Phosphor logo and an unknown brand with the sparkle", () => {
    expect(
      render(<ProviderIcon provider={{ base_url: "https://api.openai.com/v1" }} />).querySelector(
        ".ph-fill.ph-open-ai-logo",
      ),
    ).not.toBeNull();
    act(() => unmount?.());
    expect(
      render(<ProviderIcon provider={{ base_url: "https://api.typesafe.ai/v1" }} />).querySelector(".ph-sparkle"),
    ).not.toBeNull();
  });

  it("draws any other brand as its logo", () => {
    const logo = render(<ProviderIcon provider={{ base_url: "https://api.anthropic.com" }} />).firstElementChild;
    expect(logo?.tagName).toBe("SPAN");
    expect(logo?.className).toContain("anthropic");
  });
});
