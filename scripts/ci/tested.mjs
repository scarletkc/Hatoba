// Says whether the pull request that a push to main merged already passed CI on the same files, so
// that CI can skip the tests on main that would repeat that run. Anything it cannot confirm, such as
// a direct push or a pull request merged before its CI passed, counts as not tested.
// Usage: node scripts/ci/tested.mjs, with GITHUB_TOKEN set and HEAD and its parent checked out.
// Prints tested=true or tested=false for HEAD, and in GitHub Actions also appends it to
// $GITHUB_OUTPUT.
import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { ROOT, git } from "../lib/git.mjs";
import { githubClient } from "../lib/github.mjs";

/** Of `pulls`, the pull requests the API lists for commit `sha`, the one whose merge made it. */
export const mergedPull = (pulls, sha) => pulls.find((pull) => pull.merge_commit_sha === sha) ?? null;

/**
 * Why the CI runs on the last commit of pull request `pull` do not cover a commit with the files of
 * `tree`, or null when they do. `behindBy` is how many commits of main, as it was before the merge,
 * the pull request lacked. With none, its CI ran on the pull request's own files, whatever main was
 * at the time. `runs` are the CI runs on its last commit, as the API returns them.
 */
export function uncovered({ pull, tree, behindBy, runs }) {
  const name = `#${pull.number}`;
  if (behindBy !== 0) return `${name} was merged ${behindBy} commits behind main, so its CI ran on other files.`;
  const [run] = runs
    .filter(({ event }) => event === "pull_request")
    .sort((a, b) => b.created_at.localeCompare(a.created_at));
  if (!run) return `CI has not run on the last commit of ${name}.`;
  if (run.conclusion !== "success") return `The last CI run on ${name} has not passed: ${run.html_url}`;
  if (run.head_commit?.tree_id !== tree) return `The last CI run on ${name} was on other files: ${run.html_url}`;
  return null;
}

/** Why the tests still have to run on HEAD of `root`, or null when its pull request passed them. */
async function untested(repo, token, root = ROOT) {
  const { request } = githubClient(token, { userAgent: "hatoba-ci" });
  const get = (path) => request("GET", `/repos/${repo}/${path}`);
  const sha = git(["rev-parse", "HEAD"], root);
  const pull = mergedPull(await get(`commits/${sha}/pulls`), sha);
  if (!pull) return `No pull request was merged as ${sha.slice(0, 7)}.`;
  const head = pull.head.sha;
  const [comparison, { workflow_runs: runs }] = await Promise.all([
    get(`compare/${git(["rev-parse", "HEAD^"], root)}...${head}`),
    get(`actions/workflows/ci.yml/runs?head_sha=${head}&event=pull_request`),
  ]);
  const tree = git(["rev-parse", "HEAD^{tree}"], root);
  const reason = uncovered({ pull, tree, behindBy: comparison.behind_by, runs });
  if (reason === null) console.log(`#${pull.number} passed CI on the files of ${sha.slice(0, 7)}.`);
  return reason;
}

async function main() {
  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    console.error("GITHUB_TOKEN is not set");
    return 2;
  }
  let reason;
  try {
    reason = await untested(process.env.GITHUB_REPOSITORY ?? "scarletkc/Hatoba", token);
  } catch (error) {
    // The tests run when in doubt.
    reason = `Could not tell whether a pull request tested this commit: ${error.message}`;
  }
  if (reason !== null) console.log(reason);
  const output = `tested=${reason === null}`;
  console.log(output);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${output}\n`);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = await main();
}
