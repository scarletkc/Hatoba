// Closes a pull request from outside the project that skips what CONTRIBUTING.md asks of it, with
// a comment that lists what is missing, and reopens it once the author fixes that. The PR policy
// workflow runs it on pull_request_target and issue_comment events. It reads the pull request
// through the API and never checks out or runs the pull request's code.
// Usage: node scripts/ci/pr-policy.mjs, in that workflow (GITHUB_TOKEN and the event come from the
// runner), or node scripts/ci/pr-policy.mjs --dry-run NUMBER with GITHUB_TOKEN set, which only
// prints what the check finds.
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";

const GUIDE = "https://github.com/scarletkc/Hatoba/blob/main/CONTRIBUTING.md";
/** Marks the comment this check writes, so a later run edits it instead of adding another. */
const MARKER = "<!-- pr-policy";
const FAILED = `${MARKER}: failed -->`;
const PASSED = `${MARKER}: passed -->`;
const TEMPLATE = "https://github.com/scarletkc/Hatoba/blob/main/.github/pull_request_template.md";
const BOT = "github-actions[bot]";
/** Authors whose pull requests the check leaves alone. */
const MAINTAINERS = new Set(["OWNER", "MEMBER", "COLLABORATOR"]);
/** The labels that open an issue to every contributor. */
const OPEN_LABELS = new Set(["good first issue", "help wanted"]);
/** The sections of the pull request template that every description fills in. */
const SECTIONS = ["Context", "Changes", "Validation", "Breaking changes"];
/** The checklist of the pull request template, by a phrase from each item. */
const CHECKLIST = [
  { phrase: "contribution guide", text: "I have read the contribution guide" },
  { phrase: "linked issue", text: "The linked issue is labeled `good first issue` or `help wanted`, was opened by me, or is assigned to me", issue: true },
  { phrase: "reviewed the full diff", text: "I have reviewed the full diff" },
];
/** A pull request without an issue may fix documentation in at most this many changed lines. */
const SMALL_FIX_LINES = 20;

/**
 * `text` without HTML comments, as GitHub shows it: a comment left open hides the rest. Runs until
 * nothing changes, so removing one comment cannot leave another behind, as `<!<!-- -->--` would.
 */
function stripComments(text) {
  for (;;) {
    const stripped = text.replace(/<!--[\s\S]*?(?:-->|$)/g, "");
    if (stripped === text) return text;
    text = stripped;
  }
}
/** "A", "A and B", "A, B and C". */
const list = (items) => (items.length > 1 ? `${items.slice(0, -1).join(", ")} and ${items.at(-1)}` : items[0]);

/** The text under each `##` or `###` heading of `body`, by heading in lower case, without comments. */
export function sections(body) {
  const found = new Map();
  let current = null;
  for (const line of stripComments(body).split(/\r?\n/)) {
    const heading = /^#{2,3}\s+(.+?)\s*#*\s*$/.exec(line);
    if (heading) {
      current = heading[1].toLowerCase();
      found.set(current, "");
    } else if (current !== null) {
      found.set(current, `${found.get(current)}${line}\n`);
    }
  }
  return new Map([...found].map(([name, text]) => [name, text.trim()]));
}

/** The checklist items of the template that `body` has ticked, by their phrase. */
export function ticked(body) {
  const lines = stripComments(body)
    .split(/\r?\n/)
    .filter((line) => /^\s*[-*]\s*\[[xX]\]/.test(line))
    .map((line) => line.toLowerCase());
  return new Set(CHECKLIST.filter((item) => lines.some((line) => line.includes(item.phrase))).map((item) => item.phrase));
}

/** The issue numbers `body` links with a closing keyword or `Refs`, in the order they appear. */
export function linkedIssues(body, repo) {
  const own = repo.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const target = `(?:#|${own}#|https://github\\.com/${own}/issues/)(\\d+)`;
  const keyword = "(?:close[sd]?|fix(?:e[sd])?|resolve[sd]?|refs?)";
  const pattern = new RegExp(`\\b${keyword}:?\\s+${target}\\b`, "gi");
  const numbers = [...stripComments(body).matchAll(pattern)].map((m) => Number(m[1]));
  return [...new Set(numbers)];
}

/** Whether a change to the file at `path` changes the desktop app's UI, which needs screenshots. */
export function isUiFile(path) {
  if (!path.startsWith("apps/desktop/")) return false;
  if (path === "apps/desktop/index.html" || path.startsWith("apps/desktop/src/i18n/locales/")) return true;
  if (!path.startsWith("apps/desktop/src/") || /\.test\.tsx?$/.test(path)) return false;
  return path.endsWith(".tsx") || path.endsWith(".css");
}

/** Whether the files, as the pull request files API lists them, are a small fix to the docs. */
export function isSmallDocsFix(files) {
  const lines = files.reduce((sum, f) => sum + f.additions + f.deletions, 0);
  return files.length > 0 && files.every((f) => f.filename.endsWith(".md")) && lines <= SMALL_FIX_LINES;
}

/** Whether `text` shows an image or a video: Markdown, an `<img>` tag, or an uploaded attachment. */
export const hasImage = (text) => /!\[[^\]]*\]\([^)\s]+|<img\s|https:\/\/github\.com\/user-attachments\/assets\//i.test(stripComments(text));

/**
 * What the pull request lacks, as instructions to its author; empty when nothing is missing.
 * `issues` maps each linked number to `{ pullRequest, labels, user, assignees }` from the issues
 * API, or to null when the repository has no such issue. `comments` are the author's comments.
 */
export function problems({ body, author, files, issues, comments }) {
  const found = [];
  const text = body ?? "";
  const needsIssue = !isSmallDocsFix(files);

  const done = ticked(text);
  const required = CHECKLIST.filter((item) => needsIssue || !item.issue);
  const open = required.filter((item) => !done.has(item.phrase));
  if (open.length === required.length) {
    found.push(`Add the checklist from the [pull request template](${TEMPLATE}) and tick each item.`);
  } else if (open.length > 0) {
    found.push(`Tick ${list(open.map((item) => `"${item.text}"`))} in the checklist.`);
  }

  const written = sections(text);
  const missing = SECTIONS.filter((name) => !written.has(name.toLowerCase()));
  const empty = SECTIONS.filter((name) => written.get(name.toLowerCase()) === "");
  if (missing.length > 0) found.push(`Add the ${list(missing)} ${missing.length > 1 ? "sections" : "section"} of the template.`);
  if (empty.length > 0) found.push(`Fill in the ${list(empty)} ${empty.length > 1 ? "sections" : "section"}.`);

  if (needsIssue) {
    const linked = [...issues.keys()];
    if (linked.length === 0) {
      found.push(
        `Link the issue with \`Closes #N\`, or \`Refs #N\` when this addresses part of it. Open one first if there is none ([Start with an issue](${GUIDE}#start-with-an-issue)).`,
      );
    } else if (!linked.some((n) => openTo(issues.get(n), author))) {
      for (const n of linked) found.push(issueProblem(n, issues.get(n)));
    }
  }

  const ui = files.map((f) => f.filename).filter(isUiFile);
  if (ui.length > 0 && !hasImage(written.get("screenshots") ?? "") && !comments.some((c) => hasImage(c.body ?? ""))) {
    found.push(
      `This changes the UI (\`${ui[0]}\`${ui.length > 1 ? ` and ${ui.length - 1} more` : ""}), so add screenshots of the running app to the Screenshots section or in a comment ([Add screenshots for UI changes](${GUIDE}#add-screenshots-for-ui-changes)).`,
    );
  }
  return found;
}

/** Whether a contributor may work on the issue: open to everyone, opened by them, or assigned to them. */
function openTo(issue, author) {
  if (!issue || issue.pullRequest) return false;
  return issue.labels.some((l) => OPEN_LABELS.has(l)) || issue.user === author || issue.assignees.includes(author);
}

function issueProblem(n, issue) {
  if (!issue) return `#${n} is not an issue in this repository; link the issue this pull request is for.`;
  if (issue.pullRequest) return `#${n} is a pull request; link the issue this pull request is for.`;
  return `#${n} is not open to you: it is not labeled \`good first issue\` or \`help wanted\`, you did not open it, and it is not assigned to you. Ask in the issue to be assigned before working on it.`;
}

export function failureComment(found) {
  return [
    FAILED,
    `This pull request was closed automatically because it does not follow the [contribution guide](${GUIDE}) yet:`,
    "",
    ...found.map((p) => `- ${p}`),
    "",
    "Edit the description to fix these, and the check runs again and reopens the pull request once nothing is missing. Screenshots can also go in a comment of yours, which runs the check too.",
  ].join("\n");
}

const passComment = (reopened) =>
  `${PASSED}\nThe pull request now follows the [contribution guide](${GUIDE})${reopened ? ", so it is open again" : ""}.`;

function client(token) {
  const base = process.env.GITHUB_API_URL ?? "https://api.github.com";
  async function request(method, path, body, { missingOk = false } = {}) {
    const res = await fetch(`${base}${path}`, {
      method,
      headers: {
        authorization: `Bearer ${token}`,
        accept: "application/vnd.github+json",
        "x-github-api-version": "2022-11-28",
        ...(body ? { "content-type": "application/json" } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (missingOk && res.status === 404) return null;
    if (!res.ok) throw new Error(`${method} ${path}: HTTP ${res.status} ${await res.text()}`);
    return res.status === 204 ? null : res.json();
  }
  async function all(path) {
    const items = [];
    for (let page = 1; ; page++) {
      const batch = await request("GET", `${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
      items.push(...batch);
      if (batch.length < 100) return items;
    }
  }
  return { request, all };
}

/** The pull request's number, or null when the event cannot change the result. */
function eventTarget(name, event) {
  if (name === "issue_comment") {
    // Only the author's comments can bring the screenshots.
    const ok = event.issue.pull_request && event.comment.user.login === event.issue.user.login;
    return ok ? event.issue.number : null;
  }
  return event.pull_request.number;
}

/** How many times a run reads the pull request, at most, while it keeps changing under the run. */
const ROUNDS = 3;

const isOurs = (comment) => comment.user.login === BOT && comment.body.includes(MARKER);

/**
 * Checks pull request `number`, then comments on it and closes or reopens it; `dryRun` only prints.
 *
 * Runs are not queued one after another: GitHub cancels a queued run when a newer one joins the
 * queue, and the cancelled run shows as a failed check. Runs of two events can therefore overlap,
 * so every write here can be repeated, and a run reads the pull request again after acting. When
 * what the check reads changed meanwhile, as when the author edited it again, the run checks it
 * again, so the run that finishes last acts on what the pull request is now.
 */
export async function run({ repo, number, token, dryRun }) {
  const api = client(token);
  let checked = null;
  for (let round = 0; round < ROUNDS; round++) {
    const state = await read(api, repo, number);
    if (state === null) return;
    const inputs = JSON.stringify([state.pr.body, state.pr.head.sha, state.pr.draft, state.comments.map((c) => [c.id, c.body])]);
    if (inputs === checked) return;
    checked = inputs;
    const found = problems(state);
    console.log(found.length === 0 ? `#${number} follows the contribution guide.` : `#${number} is missing:\n${found.map((p) => `  - ${p}`).join("\n")}`);
    if (dryRun) return;
    await act(api, repo, number, state, found);
  }
}

/** What the check reads of the pull request, or null when the check leaves it alone. */
async function read({ request, all }, repo, number) {
  const pr = await request("GET", `/repos/${repo}/pulls/${number}`);
  const author = pr.user.login;
  if (pr.merged || pr.base.ref !== "main") return skip(`#${number} is merged or not for main; nothing to check.`);
  if (MAINTAINERS.has(pr.author_association) || pr.user.type === "Bot") {
    return skip(`#${number} is by ${author} (${pr.author_association}); the check leaves it alone.`);
  }
  if (pr.draft) return skip(`#${number} is a draft; it is checked once it is ready for review.`);

  const [files, comments, events] = await Promise.all([
    all(`/repos/${repo}/pulls/${number}/files`),
    all(`/repos/${repo}/issues/${number}/comments`),
    all(`/repos/${repo}/issues/${number}/events`),
  ]);
  // Only a maintainer can reopen someone else's pull request, and doing so decides to keep it.
  if (events.some((e) => e.event === "reopened" && e.actor && e.actor.login !== author && e.actor.login !== BOT)) {
    return skip(`#${number} was reopened by a maintainer; the check leaves it alone.`);
  }
  const issues = new Map();
  for (const n of linkedIssues(pr.body ?? "", repo)) {
    const issue = await request("GET", `/repos/${repo}/issues/${n}`, undefined, { missingOk: true });
    issues.set(
      n,
      issue && {
        pullRequest: Boolean(issue.pull_request),
        labels: issue.labels.map((l) => (typeof l === "string" ? l : l.name)),
        user: issue.user.login,
        assignees: issue.assignees.map((a) => a.login),
      },
    );
  }
  const authorComments = comments.filter((c) => c.user.login === author);
  return { pr, body: pr.body, author, files, issues, comments: authorComments, ours: comments.filter(isOurs) };
}

function skip(reason) {
  console.log(reason);
  return null;
}

async function act(api, repo, number, { pr, ours }, found) {
  const { request } = api;
  if (found.length > 0) {
    await settleComment(api, repo, number, ours, failureComment(found));
    if (pr.state === "open") await request("PATCH", `/repos/${repo}/pulls/${number}`, { state: "closed" });
    return;
  }
  if (ours.length === 0) return;
  // Reopens only what this check closed, not a pull request its author or a maintainer closed.
  const closedByCheck = pr.state === "closed" && (await request("GET", `/repos/${repo}/issues/${number}`)).closed_by?.login === BOT;
  if (closedByCheck) await request("PATCH", `/repos/${repo}/pulls/${number}`, { state: "open" });
  const kept = oldest(ours);
  await settleComment(api, repo, number, ours, kept.body.startsWith(FAILED) ? passComment(closedByCheck) : kept.body);
}

const oldest = (comments) => comments.reduce((a, b) => (b.id < a.id ? b : a));

/**
 * Leaves one comment of this check, with `body`: edits the oldest, or adds one when there is none,
 * and deletes the copies that a run alongside this one added. Every run keeps the same oldest one.
 */
async function settleComment({ request, all }, repo, number, ours, body) {
  if (ours.length === 0) {
    await request("POST", `/repos/${repo}/issues/${number}/comments`, { body });
    ours = (await all(`/repos/${repo}/issues/${number}/comments`)).filter(isOurs);
    if (ours.length === 0) return;
  }
  const kept = oldest(ours);
  for (const copy of ours) {
    if (copy !== kept) await request("DELETE", `/repos/${repo}/issues/comments/${copy.id}`, undefined, { missingOk: true });
  }
  if (kept.body !== body) await request("PATCH", `/repos/${repo}/issues/comments/${kept.id}`, { body }, { missingOk: true });
}

async function main(argv) {
  const { values, positionals } = parseArgs({ args: argv, options: { "dry-run": { type: "boolean" } }, allowPositionals: true });
  const token = process.env.GITHUB_TOKEN;
  const repo = process.env.GITHUB_REPOSITORY ?? "scarletkc/Hatoba";
  if (!token) {
    console.error("GITHUB_TOKEN is not set");
    return 2;
  }
  if (values["dry-run"]) {
    if (positionals.length !== 1) {
      console.error("usage: node scripts/ci/pr-policy.mjs --dry-run NUMBER");
      return 2;
    }
    await run({ repo, number: Number(positionals[0]), token, dryRun: true });
    return 0;
  }
  const event = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
  const number = eventTarget(process.env.GITHUB_EVENT_NAME, event);
  if (number === null) {
    console.log("This event cannot change the result.");
    return 0;
  }
  await run({ repo, number, token, dryRun: false });
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = await main(process.argv.slice(2));
}
