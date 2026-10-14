import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createServer } from "node:http";
import { test } from "node:test";
import { hasImage, isSmallDocsFix, isUiFile, linkedIssues, problems, run, sections, ticked } from "./pr-policy.mjs";

const REPO = "scarletkc/Hatoba";
const TEMPLATE = readFileSync(new URL("../../.github/pull_request_template.md", import.meta.url), "utf8").replaceAll("\r\n", "\n");

/** The template, filled in and ticked, linking issue 42. */
const FILLED = TEMPLATE.replaceAll("- [ ]", "- [x]")
  .replace("## Context\n", "## Context\n\nCloses #42. The terminal dropped IME input.\n")
  .replace("## Changes\n", "## Changes\n\nKeep the composition across tab switches.\n")
  .replace("## Validation\n", "## Validation\n\n`pnpm test` passes.\n")
  .replace("## Breaking changes\n", "## Breaking changes\n\nNone.\n");

const code = (filename, lines = 10) => ({ filename, additions: lines, deletions: 0 });
const issue = (fields = {}) => ({ pullRequest: false, labels: [], user: "maintainer", assignees: [], ...fields });
const check = ({ body = FILLED, files = [code("crates/hatoba-core/src/lib.rs")], issues, comments = [] } = {}) =>
  problems({ body, author: "alice", files, issues: issues ?? new Map([[42, issue({ labels: ["help wanted"] })]]), comments });

test("the template fills in to a description with nothing missing", () => {
  assert.deepEqual(check(), []);
});

test("the template as it comes, unticked and empty, misses every part", () => {
  const found = check({ body: TEMPLATE, issues: new Map() });
  assert.equal(found.length, 3);
  assert.match(found[0], /^Add the checklist/);
  assert.match(found[1], /^Fill in the Context, Changes, Validation and Breaking changes sections/);
  assert.match(found[2], /^Link the issue/);
});

test("a description without the template misses its checklist and sections", () => {
  const found = check({ body: "Implements the tray. Refs #42", issues: undefined });
  assert.match(found[0], /^Add the checklist/);
  assert.match(found[1], /^Add the Context, Changes, Validation and Breaking changes sections/);
  assert.equal(found.length, 2);
  assert.equal(check({ body: null, issues: new Map() }).length, 3);
});

test("each unticked item of the checklist is named", () => {
  const body = FILLED.replace("- [x] I have reviewed", "- [ ] I have reviewed");
  assert.deepEqual(check({ body }), ['Tick "I have reviewed the full diff" in the checklist.']);
});

test("template comments do not count as content", () => {
  assert.equal(sections(TEMPLATE).get("context"), "");
  assert.deepEqual(ticked("<!-- - [x] I have read the contribution guide -->"), new Set());
  assert.deepEqual(linkedIssues("<!-- Closes #1 -->", REPO), []);
  // GitHub hides the rest of a comment left open, and what one removal leaves behind.
  assert.deepEqual(linkedIssues("Text <!-- Closes #2", REPO), []);
  assert.deepEqual(linkedIssues("<!<!-- -->-- Closes #3 -->", REPO), []);
});

test("issues are linked with a closing keyword or Refs, in any form GitHub links", () => {
  assert.deepEqual(linkedIssues("Closes #1, fixes #2. Resolved: #3\nrefs #4 and Refs #4", REPO), [1, 2, 3, 4]);
  assert.deepEqual(linkedIssues(`Fixes ${REPO}#5 and closes https://github.com/${REPO}/issues/6`, REPO), [5, 6]);
  assert.deepEqual(linkedIssues("Implements #7, see #8. Discloses #9", REPO), []);
  assert.deepEqual(linkedIssues("Fixes other/repo#10", REPO), []);
});

test("an issue is open to its author, its assignee, and everyone when labeled for it", () => {
  for (const linked of [issue({ labels: ["good first issue"] }), issue({ labels: ["help wanted"] }), issue({ user: "alice" }), issue({ assignees: ["alice"] })]) {
    assert.deepEqual(check({ issues: new Map([[42, linked]]) }), [], JSON.stringify(linked));
  }
});

test("an issue that is not open to the author, a pull request, or a missing issue does not count", () => {
  assert.match(check({ issues: new Map([[42, issue()]]) })[0], /^#42 is not open to you/);
  assert.match(check({ issues: new Map([[42, issue({ pullRequest: true, user: "alice" })]]) })[0], /^#42 is a pull request/);
  assert.match(check({ issues: new Map([[42, null]]) })[0], /^#42 is not an issue/);
});

test("one issue open to the author is enough", () => {
  assert.deepEqual(check({ issues: new Map([[41, issue()], [42, issue({ user: "alice" })]]) }), []);
});

test("a small fix to the docs needs no issue, and no tick for one", () => {
  const body = FILLED.replace("Closes #42. ", "").replace("- [x] The linked issue", "- [ ] The linked issue");
  const docs = [code("README.md", 2), code("docs/development.md", 3)];
  assert.ok(isSmallDocsFix(docs));
  assert.deepEqual(check({ body, files: docs, issues: new Map() }), []);
  assert.ok(!isSmallDocsFix([code("docs/hatoba-spec.md", 21)]));
  assert.ok(!isSmallDocsFix([code("README.md", 1), code("apps/desktop/src/App.tsx", 1)]));
  assert.ok(!isSmallDocsFix([]));
});

test("components, styles and copy of the desktop app are UI, tests and logic are not", () => {
  for (const path of [
    "apps/desktop/src/app/App.tsx",
    "apps/desktop/src/app/App.module.css",
    "apps/desktop/src/styles/tokens.css",
    "apps/desktop/src/i18n/locales/settings.ts",
    "apps/desktop/index.html",
  ]) {
    assert.ok(isUiFile(path), path);
  }
  for (const path of [
    "apps/desktop/src/components/overlay.test.tsx",
    "apps/desktop/src/app/store.ts",
    "apps/desktop/src-tauri/src/lib.rs",
    "apps/site/src/pages/index.astro",
    "docs/design/README.md",
  ]) {
    assert.ok(!isUiFile(path), path);
  }
});

test("a UI change needs an image in the Screenshots section or an author's comment", () => {
  const files = [code("apps/desktop/src/features/settings/GeneralPane.tsx")];
  assert.match(check({ files })[0], /^This changes the UI \(`apps\/desktop\/src\/features\/settings\/GeneralPane.tsx`\)/);
  const shot = "![Settings, Windows, dark](https://github.com/user-attachments/assets/0f6e)";
  assert.deepEqual(check({ files, body: FILLED.replace("## Screenshots\n", `## Screenshots\n\n${shot}\n`) }), []);
  assert.deepEqual(check({ files, comments: [{ body: `Light theme:\n${shot}` }] }), []);
  // An image elsewhere in the description is not in the Screenshots section.
  assert.equal(check({ files, body: FILLED.replace("## Changes\n", `## Changes\n\n${shot}\n`) }).length, 1);
});

test("images come as Markdown, as img tags, or as attachment links", () => {
  assert.ok(hasImage("![a](https://example.com/a.png)"));
  assert.ok(hasImage('<img width="800" alt="Settings" src="https://example.com/a.png" />'));
  assert.ok(hasImage("https://github.com/user-attachments/assets/2c3d-video"));
  assert.ok(!hasImage("Screenshots to follow."));
  assert.ok(!hasImage("<!-- ![a](https://example.com/a.png) -->"));
});

/**
 * Runs the check against a stand-in for the GitHub API that serves pull request 7 with `comments`
 * and `events`. `pr` is the pull request, or a list of what each read of it returns in turn, the
 * last one staying. Comments can be added, edited and deleted. Returns the writes it received as
 * "METHOD path" with their bodies, the comments it ends with, and how often the pull request was read.
 */
async function runAgainst({ pr, comments = [], events = [], closedBy = null }) {
  const writes = [];
  const reads = Array.isArray(pr) ? [...pr] : [pr];
  let prReads = 0;
  let nextId = 100;
  const stored = comments.map((c) => ({ ...c }));
  const get = (path) => {
    switch (path) {
      case "/repos/o/r/pulls/7":
        prReads++;
        return reads.length > 1 ? reads.shift() : reads[0];
      case "/repos/o/r/pulls/7/files":
        return [code("apps/desktop/src/app/App.tsx")];
      case "/repos/o/r/issues/7/comments":
        return stored;
      case "/repos/o/r/issues/7/events":
        return events;
      case "/repos/o/r/issues/7":
        return { closed_by: closedBy && { login: closedBy } };
      case "/repos/o/r/issues/42":
        return { labels: [{ name: "help wanted" }], user: { login: "maintainer" }, assignees: [] };
      default:
        return undefined;
    }
  };
  const answer = (method, path, body) => {
    if (method === "GET") return get(path);
    if (method === "POST" && path === "/repos/o/r/issues/7/comments") {
      const comment = { id: nextId++, user: { login: "github-actions[bot]" }, body: body.body };
      stored.push(comment);
      return comment;
    }
    const id = /^\/repos\/o\/r\/issues\/comments\/(\d+)$/.exec(path)?.[1];
    if (id === undefined) return {};
    const at = stored.findIndex((c) => String(c.id) === id);
    if (at < 0) return undefined;
    if (method === "DELETE") return stored.splice(at, 1) && null;
    stored[at].body = body.body;
    return stored[at];
  };
  const server = createServer((req, res) => {
    let data = "";
    req.on("data", (chunk) => (data += chunk));
    req.on("end", () => {
      const path = req.url.split("?")[0];
      const body = data ? JSON.parse(data) : undefined;
      if (req.method !== "GET") writes.push([`${req.method} ${path}`, body]);
      const result = answer(req.method, path, body);
      if (result === null) return res.writeHead(204).end();
      res.writeHead(result === undefined ? 404 : 200, { "content-type": "application/json" });
      res.end(JSON.stringify(result ?? { message: "Not Found" }));
    });
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  process.env.GITHUB_API_URL = `http://127.0.0.1:${server.address().port}`;
  try {
    await run({ repo: "o/r", number: 7, token: "t", dryRun: false });
  } finally {
    server.close();
    delete process.env.GITHUB_API_URL;
  }
  return { writes, comments: stored, prReads };
}

const pull = (fields = {}) => ({
  number: 7,
  state: "open",
  draft: false,
  merged: false,
  head: { sha: "c0ffee" },
  base: { ref: "main" },
  user: { login: "alice", type: "User" },
  author_association: "FIRST_TIME_CONTRIBUTOR",
  body: FILLED,
  ...fields,
});
const shot = "![Settings](https://github.com/user-attachments/assets/0f6e)";
const failed = { id: 9, user: { login: "github-actions[bot]" }, body: "<!-- pr-policy: failed -->\nThis pull request was closed" };

test("a pull request that misses something gets a comment and is closed", async () => {
  const { writes } = await runAgainst({ pr: pull() });
  assert.deepEqual(writes.map(([what]) => what), ["POST /repos/o/r/issues/7/comments", "PATCH /repos/o/r/pulls/7"]);
  assert.match(writes[0][1].body, /^<!-- pr-policy: failed -->\n/);
  assert.match(writes[0][1].body, /This changes the UI/);
  assert.deepEqual(writes[1][1], { state: "closed" });
});

test("a later run edits its comment instead of adding another", async () => {
  const { writes } = await runAgainst({ pr: pull({ state: "closed" }), comments: [failed], closedBy: "github-actions[bot]" });
  assert.deepEqual(writes.map(([what]) => what), ["PATCH /repos/o/r/issues/comments/9"]);
});

test("a pull request the check closed reopens once the author adds what was missing", async () => {
  const comments = [failed, { user: { login: "alice" }, body: shot }];
  const { writes } = await runAgainst({ pr: pull({ state: "closed" }), comments, closedBy: "github-actions[bot]" });
  assert.deepEqual(writes.map(([what]) => what), ["PATCH /repos/o/r/pulls/7", "PATCH /repos/o/r/issues/comments/9"]);
  assert.deepEqual(writes[0][1], { state: "open" });
  assert.match(writes[1][1].body, /^<!-- pr-policy: passed -->\n.*so it is open again\.$/);
});

test("a pull request its author closed stays closed", async () => {
  const comments = [failed, { user: { login: "alice" }, body: shot }];
  const { writes } = await runAgainst({ pr: pull({ state: "closed" }), comments, closedBy: "alice" });
  assert.deepEqual(writes.map(([what]) => what), ["PATCH /repos/o/r/issues/comments/9"]);
  assert.doesNotMatch(writes[0][1].body, /open again/);
});

test("a screenshot from someone else does not count", async () => {
  const { writes } = await runAgainst({ pr: pull(), comments: [{ user: { login: "bob" }, body: shot }] });
  assert.deepEqual(writes.at(-1), ["PATCH /repos/o/r/pulls/7", { state: "closed" }]);
});

test("maintainers, bots, drafts, and pull requests a maintainer reopened are left alone", async () => {
  for (const pr of [pull({ author_association: "OWNER" }), pull({ author_association: "COLLABORATOR" }), pull({ user: { login: "dependabot[bot]", type: "Bot" } }), pull({ draft: true })]) {
    assert.deepEqual((await runAgainst({ pr })).writes, [], JSON.stringify(pr));
  }
  const events = [{ event: "closed", actor: { login: "github-actions[bot]" } }, { event: "reopened", actor: { login: "scarletkc" } }];
  assert.deepEqual((await runAgainst({ pr: pull(), events })).writes, []);
  // The author reopening their own pull request is checked as usual.
  const own = [{ event: "reopened", actor: { login: "alice" } }];
  assert.equal((await runAgainst({ pr: pull(), events: own })).writes.length, 2);
});

test("a run reads the pull request again after acting, and stops when nothing changed", async () => {
  const { writes, prReads } = await runAgainst({ pr: pull() });
  assert.equal(writes.length, 2);
  assert.equal(prReads, 2);
});

test("a run checks again when the pull request changed while it acted", async () => {
  // Another run's event: the author added the screenshots after this run read the pull request.
  const fixed = pull({ state: "closed", body: FILLED.replace("## Screenshots\n", `## Screenshots\n\n${shot}\n`) });
  const { writes, comments } = await runAgainst({ pr: [pull(), fixed], closedBy: "github-actions[bot]" });
  assert.deepEqual(
    writes.map(([what, body]) => `${what} ${body?.state ?? ""}`.trim()),
    ["POST /repos/o/r/issues/7/comments", "PATCH /repos/o/r/pulls/7 closed", "PATCH /repos/o/r/pulls/7 open", "PATCH /repos/o/r/issues/comments/100"],
  );
  assert.equal(comments.length, 1);
  assert.match(comments[0].body, /^<!-- pr-policy: passed -->\n.*so it is open again\.$/);
});

test("copies of the comment that runs alongside added are deleted, and the oldest stays", async () => {
  const copy = { ...failed, id: 12 };
  const { writes, comments } = await runAgainst({ pr: pull({ state: "closed" }), comments: [copy, failed], closedBy: "github-actions[bot]" });
  assert.deepEqual(writes.map(([what]) => what), ["DELETE /repos/o/r/issues/comments/12", "PATCH /repos/o/r/issues/comments/9"]);
  assert.deepEqual(comments.map((c) => c.id), [9]);
  assert.match(comments[0].body, /This changes the UI/);
});

test("the workflow does not queue runs, and skips pull requests the check leaves alone", () => {
  const workflow = readFileSync(new URL("../../.github/workflows/pr-policy.yml", import.meta.url), "utf8");
  // A run that a newer one replaced in the queue would show as a failed check.
  assert.doesNotMatch(workflow, /^\s*concurrency:/m);
  for (const event of ["github.event.pull_request", "github.event.issue"]) {
    assert.ok(workflow.includes(`!contains(fromJSON('["OWNER", "MEMBER", "COLLABORATOR"]'), ${event}.author_association)`), event);
    assert.ok(workflow.includes(`${event}.user.type != 'Bot'`), event);
  }
});
