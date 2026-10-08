// Checks that a pull request title is a Conventional Commits header.
// Usage: node scripts/ci/pr-title.mjs "<title>", or with no argument in a pull_request workflow.
import { readFileSync } from "node:fs";

// Keep in sync with the commit types in CONTRIBUTING.md.
const TYPES = ["feat", "fix", "perf", "refactor", "docs", "test", "build", "ci", "style", "chore", "revert"];
const MAX_LENGTH = 72;
const GUIDE = "https://github.com/scarletkc/Hatoba/blob/main/CONTRIBUTING.md#write-commits";
const HEADER = /^(?<type>[A-Za-z]+)(?:\((?<scope>[^()]*)\))?!?: (?<description>.*)$/;
const SCOPE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;

function problems(title) {
  const errors = [];
  const length = [...title].length;
  if (length > MAX_LENGTH) {
    errors.push(`Shorten the title to at most ${MAX_LENGTH} characters; it has ${length}.`);
  }
  const match = HEADER.exec(title);
  if (match === null) {
    errors.push("Use the format <type>(<scope>): <description>; the scope and a ! before the colon are optional.");
    return errors;
  }
  const { type, scope, description } = match.groups;
  if (!TYPES.includes(type)) {
    errors.push(`Replace the type '${type}' with one of: ${TYPES.join(", ")}.`);
  }
  if (scope !== undefined && !SCOPE.test(scope)) {
    errors.push("Write the scope in lowercase letters and digits separated by single hyphens, such as desktop.");
  }
  if (!description || description !== description.trim()) {
    errors.push("Put exactly one space after the colon, followed by the description.");
  } else if (/^[A-Z][a-z]/.test(description)) {
    errors.push("Start the description with a lowercase word; identifiers keep their case.");
  }
  if (description.endsWith(".")) {
    errors.push("Remove the trailing period.");
  }
  return errors;
}

function main(argv) {
  let title;
  if (argv.length > 0) {
    title = argv[0];
  } else if (process.env.GITHUB_EVENT_PATH) {
    title = JSON.parse(readFileSync(process.env.GITHUB_EVENT_PATH, "utf8")).pull_request.title;
  } else {
    console.error("usage: node scripts/ci/pr-title.mjs TITLE (or run in a pull_request workflow)");
    return 2;
  }
  const shown = title.split(/\r?\n/).join(" ");
  const errors = problems(title);
  if (errors.length === 0) {
    console.log(`Title follows Conventional Commits: ${shown}`);
    return 0;
  }
  const prefix = process.env.GITHUB_ACTIONS === "true" ? "::error title=PR title::" : "error: ";
  console.log(`Title does not follow Conventional Commits: ${shown}`);
  for (const error of errors) {
    console.log(prefix + error);
  }
  console.log(`Edit the pull request title; this check runs again after the edit. Format: ${GUIDE}`);
  return 1;
}

process.exitCode = main(process.argv.slice(2));
