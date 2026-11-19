import { ROOT, git } from "../lib/git.mjs";
import { githubClient } from "../lib/github.mjs";
import { SIGNING_ENVIRONMENT } from "./minisign.mjs";
import { onlyVersionChanged } from "./version.mjs";

/** Keep in sync with the environment of the publish job in release.yml. */
export const ENVIRONMENT = "release";

// Keep in sync with paths-ignore under push in ci.yml: CI does not run on commits that change only
// these files.
export const skippedByCi = (path) => path.endsWith(".md") || path.startsWith("docs/");

/**
 * Whether a CI pass on `tested` covers `sha`: the same commit, or an ancestor that differs from it
 * only in files CI skips and in the release version. The CI run before a release pull request then
 * covers its commit, so a release need not wait for CI on main.
 */
export function ciCovers(tested, sha, root = ROOT) {
  if (tested === sha) return true;
  try {
    git(["merge-base", "--is-ancestor", tested, sha], root);
  } catch {
    return false;
  }
  return git(["diff", "--name-only", "--no-renames", tested, sha], root)
    .split("\n")
    .filter(Boolean)
    .every((path) => skippedByCi(path) || onlyVersionChanged(path, tested, sha, root));
}

export async function github(path, { allowMissing = false } = {}) {
  const token = process.env.GH_TOKEN;
  if (!token) throw new Error("Set GH_TOKEN to a GitHub token that can read this repository.");
  return githubClient(token, { userAgent: "hatoba-release" }).request("GET", `/repos/${path}`, undefined, {
    missingOk: allowMissing,
  });
}

/** How long the gate waits for CI that would cover the release commit. Keep below the Preflight job's timeout. */
const CI_WAIT_MINUTES = 40;

/**
 * Of the CI runs `runs`, the one that passed and covers `sha`, or else one that would cover it and
 * is still going.
 */
export function coveringRun(runs, sha, root = ROOT) {
  const covering = runs.filter((run) => ciCovers(run.head_sha, sha, root));
  return (
    covering.find((run) => run.conclusion === "success") ?? covering.find((run) => run.status !== "completed") ?? null
  );
}

export async function gate(repository, root = ROOT) {
  const sha = git(["rev-parse", "HEAD"], root);
  const deadline = Date.now() + CI_WAIT_MINUTES * 60_000;
  let passed = null;
  for (;;) {
    // The release commit's own runs first, which the latest runs on main may no longer include.
    const run = coveringRun(
      [
        ...(await github(`${repository}/actions/workflows/ci.yml/runs?head_sha=${sha}`)).workflow_runs,
        ...(await github(`${repository}/actions/workflows/ci.yml/runs?branch=main&per_page=100`)).workflow_runs,
      ],
      sha,
      root,
    );
    if (run?.conclusion === "success") passed = run;
    if (run?.conclusion === "success" || run === null || Date.now() > deadline) break;
    console.log(`Waiting for CI on ${run.head_sha.slice(0, 7)} to finish: ${run.html_url}`);
    await new Promise((resolve) => setTimeout(resolve, 60_000));
  }
  if (!passed) {
    throw new Error(
      `CI has not passed on the release commit ${sha.slice(0, 7)}, or on an earlier commit that differs from it only in files CI skips and in the release version. Wait for CI on it to pass, or rerun the failed jobs of its CI run, then rerun this workflow.`,
    );
  }
  console.log(`CI passed on ${passed.head_sha.slice(0, 7)}: ${passed.html_url}`);
  const environment = await github(`${repository}/environments/${ENVIRONMENT}`, { allowMissing: true });
  if (!environment?.protection_rules?.some((rule) => rule.type === "required_reviewers")) {
    throw new Error(
      `The ${ENVIRONMENT} environment must require a reviewer, so that nothing is published until someone approves the run. Set it up as docs/releasing.md describes, then rerun this workflow.`,
    );
  }
  console.log(`Publishing waits for approval in the ${ENVIRONMENT} environment.`);
  const signing = await github(`${repository}/environments/${SIGNING_ENVIRONMENT}`, { allowMissing: true });
  const policies = signing?.deployment_branch_policy?.custom_branch_policies
    ? (await github(`${repository}/environments/${SIGNING_ENVIRONMENT}/deployment-branch-policies?per_page=100`))
        .branch_policies
    : [];
  if (!allowsOnlyMain(signing, policies)) {
    throw new Error(
      `The ${SIGNING_ENVIRONMENT} environment must exist and allow deployments only from the main branch, because its secrets sign updates. Set it up as docs/releasing.md describes, then rerun this workflow.`,
    );
  }
  console.log(
    `The build signs updates with the secrets of the ${SIGNING_ENVIRONMENT} environment, which only main can use.`,
  );
}

/**
 * Whether an environment, as GitHub's API returns it with its deployment branch policies, allows
 * deployments from the main branch and nothing else.
 */
export function allowsOnlyMain(environment, policies) {
  const rule = environment?.deployment_branch_policy;
  return (
    rule?.custom_branch_policies === true &&
    rule.protected_branches === false &&
    policies.length === 1 &&
    policies[0].name === "main" &&
    policies[0].type === "branch"
  );
}
