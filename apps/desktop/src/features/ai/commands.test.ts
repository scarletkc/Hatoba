import { describe, expect, it } from "vitest";
import type { McpPromptView } from "@/ipc/types";
import {
  commandDraft,
  commandItems,
  commandNames,
  findBuiltin,
  leadingCommand,
  listingText,
  listPath,
  mentionText,
  pathItems,
  promptArguments,
  replaceTrigger,
  resourceItems,
  resourceTarget,
  runsAtOnce,
  splitPath,
  triggerAt,
  type PromptCommand,
} from "./commands";

describe("triggers (AI-50, AI-51)", () => {
  it("opens the / menu only for the first word of the text, on the whole word", () => {
    expect(triggerAt("/", 1)).toEqual({ sign: "/", start: 0, end: 1, query: "", quoted: false });
    expect(triggerAt("/comp", 5)).toMatchObject({ sign: "/", query: "comp" });
    // The caret inside the word: Enter must run what the word names, not what precedes the caret.
    expect(triggerAt("/compact keep it", 2)).toEqual({ sign: "/", start: 0, end: 8, query: "compact", quoted: false });
    expect(triggerAt("/compact now", 12)).toBeNull();
    expect(triggerAt("see /etc", 8)).toBeNull();
    expect(triggerAt(" /x", 3)).toBeNull();
  });

  it("opens the @ menu at the start of a word, and not for a selection", () => {
    expect(triggerAt("@", 1)).toMatchObject({ sign: "@", start: 0, query: "" });
    expect(triggerAt("look at @etc/ng", 15)).toEqual({ sign: "@", start: 8, end: 15, query: "etc/ng", quoted: false });
    expect(triggerAt("mail me@host", 12)).toBeNull();
    expect(triggerAt("@etc ", 5)).toBeNull();
    expect(triggerAt("@etc", 1, 4)).toBeNull();
    expect(triggerAt('a @"My Docs/no', 14)).toEqual({ sign: "@", start: 2, end: 14, query: "My Docs/no", quoted: true });
    expect(triggerAt('@"done" x', 9)).toBeNull();
  });

  it("replaces the trigger's whole word, and one space after it", () => {
    const tr = triggerAt("see @et and", 7)!;
    expect(replaceTrigger("see @et and", tr, "@etc/hosts ")).toEqual({ text: "see @etc/hosts and", caret: 15 });
    const at = triggerAt("@etc/ng", 7)!;
    expect(replaceTrigger("@etc/nginx.conf more", at, "")).toEqual({ text: " more", caret: 0 });
    const quoted = triggerAt('@"My D', 6)!;
    expect(replaceTrigger('@"My Docs" x', quoted, '@"My Docs/" ')).toEqual({ text: '@"My Docs/" x', caret: 12 });
    // A later quote after a space closes some other text, not the mention.
    const open = triggerAt('check @"my d and tell me "why"', 12)!;
    expect(replaceTrigger('check @"my d and tell me "why"', open, '@"my dir/')).toEqual({ text: 'check @"my dir/ and tell me "why"', caret: 15 });
    const slash = triggerAt("/co", 3)!;
    expect(replaceTrigger("/co", slash, "/compact ").text).toBe("/compact ");
  });

  it("writes a mention in quotes when it has a space", () => {
    expect(mentionText("/etc/hosts")).toBe("@/etc/hosts ");
    expect(mentionText("My Docs/a.md")).toBe('@"My Docs/a.md" ');
    expect(mentionText("github:repo://x/README.md")).toBe("@github:repo://x/README.md ");
  });
});

describe("leading commands (AI-50)", () => {
  it("reads the name and what follows it", () => {
    expect(leadingCommand("/compact")).toEqual({ name: "compact", args: "" });
    expect(leadingCommand("  /btw what is this?\n")).toEqual({ name: "btw", args: "what is this?" });
    expect(leadingCommand("/mcp__gh__review 42\nmore")).toEqual({ name: "mcp__gh__review", args: "42\nmore" });
    expect(leadingCommand("/etc/hosts is wrong")).toBeNull();
    expect(leadingCommand("hi /compact")).toBeNull();
    expect(leadingCommand("/")).toBeNull();
  });

  it("finds built-ins by name and alias", () => {
    expect(findBuiltin("clear")?.name).toBe("clear");
    expect(findBuiltin("new")?.name).toBe("clear");
    expect(findBuiltin("reset")?.name).toBe("clear");
    expect(findBuiltin("nginx")).toBeUndefined();
  });

  it("fills a prompt's arguments in order, the last one taking the rest", () => {
    expect(promptArguments("42 the tests and docs", ["pr", "focus"])).toEqual([
      { name: "pr", value: "42" },
      { name: "focus", value: "the tests and docs" },
    ]);
    expect(promptArguments('"two words" x', ["a", "b"])).toEqual([
      { name: "a", value: "two words" },
      { name: "b", value: "x" },
    ]);
    expect(promptArguments('"all of it"', ["a"])).toEqual([{ name: "a", value: "all of it" }]);
    expect(promptArguments("", ["a"])).toEqual([]);
    expect(promptArguments("x y", [])).toEqual([]);
  });

  it("starts the box with / or a skill from the + menu, keeping what was typed", () => {
    // Slash Commands: the caret on the command's word opens the menu.
    expect(commandDraft("", null)).toEqual({ text: "/", caret: 1 });
    expect(triggerAt("/", 1)).toMatchObject({ sign: "/", query: "" });
    expect(commandDraft("  check nginx", null)).toEqual({ text: "/ check nginx", caret: 1 });
    expect(triggerAt("/ check nginx", 1)).toMatchObject({ sign: "/", query: "" });
    // A skill: its command, then what was typed, in place of a command the text started with.
    expect(commandDraft("", "nginx-ops")).toEqual({ text: "/nginx-ops ", caret: 11 });
    expect(commandDraft("reload it", "nginx-ops")).toEqual({ text: "/nginx-ops reload it", caret: 11 });
    expect(commandDraft("/btw why?", "nginx-ops")).toEqual({ text: "/nginx-ops why?", caret: 11 });
    expect(commandDraft("/btw why?", null)).toEqual({ text: "/ why?", caret: 1 });
    // A path is no command, so it stays.
    expect(commandDraft("/etc/hosts is wrong", "fix")).toEqual({ text: "/fix /etc/hosts is wrong", caret: 5 });
  });
});

describe("the / menu (AI-50)", () => {
  const prompt = (command: string, required = false): PromptCommand => ({
    serverId: "s1",
    server: "github",
    prompt: { name: command.split("__").pop()!, title: null, description: "", arguments: [{ name: "pr", description: "", required }] } satisfies McpPromptView,
    command,
  });
  // Sorted by name, as Rust lists them.
  const skills = [
    { name: "compactor", description: "named like a built-in's start" },
    { name: "model", description: "named like a built-in" },
    { name: "nginx-ops", description: "nginx" },
  ];

  it("lists built-ins, skills and prompts, names that start with the query first", () => {
    const names = (q: string) => commandItems(q, skills, [prompt("mcp__github__review")]).map((i) => `${i.kind}:${i.name}`);
    expect(names("")).toEqual([
      "builtin:btw",
      "builtin:clear",
      "builtin:compact",
      "builtin:mcp",
      "builtin:model",
      "skill:compactor",
      "skill:nginx-ops",
      "prompt:mcp__github__review",
    ]);
    expect(names("comp")).toEqual(["builtin:compact", "skill:compactor"]);
    expect(names("new")).toEqual(["builtin:clear"]);
    expect(names("review")).toEqual(["prompt:mcp__github__review"]);
    expect(names("ops")).toEqual(["skill:nginx-ops"]);
    expect(names("MCP")).toEqual(["builtin:mcp", "prompt:mcp__github__review"]);
  });

  it("runs on Enter what needs nothing after its name", () => {
    const [btw, clear] = commandItems("", [], []);
    expect(runsAtOnce(btw)).toBe(false);
    expect(runsAtOnce(clear)).toBe(true);
    expect(runsAtOnce({ kind: "skill", name: "x", skill: { name: "x", description: "" } })).toBe(true);
    expect(runsAtOnce({ kind: "prompt", name: "p", prompt: prompt("p", true) })).toBe(false);
    expect(runsAtOnce({ kind: "prompt", name: "p", prompt: prompt("p") })).toBe(true);
  });
});

describe("the @ menu (AI-51)", () => {
  it("splits a path into the directory to list and the name to complete", () => {
    expect(splitPath("")).toEqual({ dir: "", leaf: "" });
    expect(splitPath("ngi")).toEqual({ dir: "", leaf: "ngi" });
    expect(splitPath("/etc/ng")).toEqual({ dir: "/etc/", leaf: "ng" });
    expect(splitPath("/")).toEqual({ dir: "/", leaf: "" });
    expect(splitPath("~/app/")).toEqual({ dir: "~/app/", leaf: "" });
    expect(listPath("")).toBe("~");
    expect(listPath("/etc/")).toBe("/etc/");
  });

  it("completes names, hidden ones only for a leading dot", () => {
    const entries = [
      { name: ".bashrc", dir: false },
      { name: "app", dir: true },
      { name: "backup.sh", dir: false },
      { name: "MyApp", dir: true },
    ];
    expect(pathItems(entries, "").map((e) => e.name)).toEqual(["app", "backup.sh", "MyApp"]);
    expect(pathItems(entries, "ap").map((e) => e.name)).toEqual(["app", "MyApp"]);
    expect(pathItems(entries, ".").map((e) => e.name)).toEqual([".bashrc", "backup.sh"]);
    expect(listingText(entries)).toBe(".bashrc\napp/\nbackup.sh\nMyApp/");
  });

  it("matches resources by server, name, title and URI", () => {
    const r = (uri: string, name: string, title: string | null = null) => ({
      serverId: "s",
      server: "docs",
      resource: { uri, name, title, description: "", mime_type: null },
    });
    const list = [r("file:///a/runbook.md", "runbook.md", "Runbook"), r("mem://logo", "logo")];
    expect(resourceItems(list, "run").map((x) => x.resource.name)).toEqual(["runbook.md"]);
    expect(resourceItems(list, "DOCS:mem").map((x) => x.resource.name)).toEqual(["logo"]);
    expect(resourceItems(list, "").length).toBe(2);
    expect(resourceTarget(list[1])).toBe("docs:mem://logo");
  });
});

describe("prompt command names (AI-50)", () => {
  it("names prompts as MCP tools are named, numbering collisions", () => {
    expect(commandNames([["GitHub Server", ["review pr", "triage"]], ["files", ["read"]]])).toEqual([
      ["mcp__GitHub_Server__review_pr", "mcp__GitHub_Server__triage"],
      ["mcp__files__read"],
    ]);
    // Two servers of one name: the first keeps the name.
    expect(commandNames([["docs", ["a"]], ["docs", ["a"]]])).toEqual([["mcp__docs__a"], ["mcp__docs__a_2"]]);
    // A suffix never takes a name another prompt has without one, and long names keep 64 characters.
    expect(commandNames([["s", ["x", "x", "x_2"]]])).toEqual([["mcp__s__x", "mcp__s__x_3", "mcp__s__x_2"]]);
    const long = commandNames([["s", ["p".repeat(80), "p".repeat(80)]]])[0];
    expect(long.map((n) => n.length)).toEqual([64, 64]);
    expect(long[1].endsWith("_2")).toBe(true);
    // One `_` for each character that is not ASCII, as Rust counts characters.
    expect(commandNames([["é😀", ["p"]]])).toEqual([["mcp______p"]]);
  });
});
