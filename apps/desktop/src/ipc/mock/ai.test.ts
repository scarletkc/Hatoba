import { afterEach, describe, expect, it } from "vitest";
import type { AiProviderView, AiSettingsView, AiTurnContext, AiTurnEvent } from "../types";
import { createAiMock } from "./ai";

const PROVIDER = {
  id: "p",
  name: "Mock",
  models: [{ id: "m", name: "Mock model", context_window: 200_000 }],
} as unknown as AiProviderView;

const CONTEXT: AiTurnContext = {
  provider_id: "p",
  model_id: "m",
  effort: null,
  host_id: "h-prod-api",
  target: null,
  local: null,
  tab: true,
  session_id: "s1",
  disabled_mcp_servers: [],
  permission_mode: "manual",
  screen: null,
};

/** The tools the mock's first reply to `text` calls, with the page in `lang`. */
async function toolsFor(lang: string, text: string): Promise<string[]> {
  history.replaceState(null, "", `/?lang=${lang}`);
  const mock = createAiMock({
    providers: async () => [PROVIDER],
    settings: async () => ({}) as AiSettingsView,
  });
  const tools: string[] = [];
  await new Promise<void>((resolve) => {
    const onEvent = (event: AiTurnEvent) => {
      if (event.kind === "tool_call") tools.push(event.name);
      if (event.kind === "done" || event.kind === "turn_ended") resolve();
    };
    void mock.ai_send({ conversation_id: null, text, context: CONTEXT }, onEvent);
  });
  return tools;
}

afterEach(() => history.replaceState(null, "", "/"));

// The requests the website's demo suggests in each Chinese script, and the tools each one shows.
describe.each([
  [
    "zh-CN",
    [
      "磁盘还剩多少空间？",
      "屏幕上显示了什么？",
      "在 shell 里输入一条命令。",
      "搜索 nginx upstream 的报错。",
      "编辑 nginx 配置。",
    ],
  ],
  [
    "zh-TW",
    [
      "磁碟還剩多少空間？",
      "螢幕上顯示了什麼？",
      "在 shell 裡輸入一條命令。",
      "搜尋 nginx upstream 的錯誤。",
      "編輯 nginx 設定。",
    ],
  ],
])("the demo's suggestions in %s", (lang, [disk, screen, type, search, edit]) => {
  it.each([
    [disk, ["run_command"]],
    [screen, ["read_terminal"]],
    [type, ["send_input"]],
    [search, ["web_search"]],
    [edit, ["read_file", "edit_file"]],
  ])("%s calls its tools", async (text, tools) => {
    expect(await toolsFor(lang, text)).toEqual(tools);
  });
});
