// Checks, describes, packages, and publishes the release of the version in the version files.
// The Release workflow (.github/workflows/release.yml) runs each command, and docs/releasing.md
// describes the steps.
// Usage: node scripts/release/release.mjs <command> [options]
//   commit                           Print the release commit, where the release pull request set
//                                    the version
//   raised --from SHA                Print the version when it is greater than the one at SHA and
//                                    has no release tag yet
//   check                            Check the version files, the release tag, the release note, and
//                                    the updater's public key
//   gate                             Require a CI pass, an approval-gated release environment, and a
//                                    signing environment limited to selected branches
//   outputs                          Print the version, the tag, and whether it is a prerelease as
//                                    the step outputs of a GitHub Actions job
//   notes --output FILE              Write the release notes
//   dist --platform NAME --bundle DIR --output DIR
//                                    Collect the platform's installers and their updater signatures
//                                    from Tauri's bundle directory
//   publish --notes FILE --dist DIR  Write the checksums and the updater's latest.json, then create
//                                    the GitHub Release or upload the files it is missing
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { ROOT, git } from "../lib/git.mjs";
import { collectDist, publish } from "./dist.mjs";
import { gate } from "./gate.mjs";
import { updaterPubkey } from "./minisign.mjs";
import { composeNotes, previousRelease } from "./notes.mjs";
import { raisedVersion, releaseCommit, releaseOutputs, releaseTarget } from "./target.mjs";
import { NOTES_DIR, readVersion } from "./version.mjs";

export { git };
export { CHECKSUMS, INSTALLERS, LATEST, installerName, installerNames, signatureName } from "./installers.mjs";
export { SIGNING_ENVIRONMENT, TAURI_CONFIG, updaterPubkey, verifyUpdaterSignature } from "./minisign.mjs";
export { raisedVersion, releaseCommit, releaseOutputs, releaseTarget } from "./target.mjs";
export {
  INSTALL_STEPS,
  commitsSince,
  composeNotes,
  groupCommits,
  handwrittenNote,
  previousRelease,
  updateNotes,
} from "./notes.mjs";
export { ENVIRONMENT, allowsOnlyMain, ciCovers, coveringRun, skippedByCi } from "./gate.mjs";
export { assembleRelease, collectDist } from "./dist.mjs";

async function main() {
  const [command, ...args] = process.argv.slice(2);
  const { values } = parseArgs({
    args,
    options: {
      output: { type: "string" },
      platform: { type: "string" },
      bundle: { type: "string" },
      notes: { type: "string" },
      dist: { type: "string" },
      from: { type: "string" },
    },
  });
  const option = (name) => {
    if (!values[name]) throw new Error(`${command} needs --${name}.`);
    return values[name];
  };
  const repository = process.env.GITHUB_REPOSITORY ?? "scarletkc/Hatoba";
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository))
    throw new Error(`GITHUB_REPOSITORY must be OWNER/REPO; got '${repository}'.`);
  const version = readVersion(ROOT);
  switch (command) {
    case "commit":
      console.log(releaseCommit(version));
      break;
    case "raised": {
      const raised = raisedVersion(version, option("from"));
      if (raised) console.log(raised);
      break;
    }
    case "check": {
      const { tag, sha } = releaseTarget(version);
      composeNotes(version, repository);
      updaterPubkey();
      const previous = previousRelease(version);
      const source = previous ? `changes since ${previous}` : `notes only from ${NOTES_DIR}/${version}.md`;
      console.log(`Ready to release Hatoba ${tag} from ${sha.slice(0, 7)}, with ${source}.`);
      break;
    }
    case "gate":
      await gate(repository);
      break;
    case "outputs":
      process.stdout.write(releaseOutputs(version));
      break;
    case "notes": {
      const output = option("output");
      mkdirSync(dirname(output), { recursive: true });
      writeFileSync(output, composeNotes(version, repository));
      console.log(`Wrote ${output}`);
      break;
    }
    case "dist": {
      const output = option("output");
      const files = collectDist(version, option("platform"), option("bundle"), output, { pubkey: updaterPubkey() });
      console.log(`Collected ${files.join(", ")} in ${output}`);
      break;
    }
    case "publish":
      await publish(version, repository, option("notes"), option("dist"));
      break;
    default:
      throw new Error(
        `Unknown command '${command ?? ""}'. Use commit, raised, check, gate, outputs, notes, dist, or publish.`,
      );
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
