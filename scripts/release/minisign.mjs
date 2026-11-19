import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT } from "../lib/git.mjs";

/** Keep in sync with the environment of the build job in release.yml, which holds the update signing key. */
export const SIGNING_ENVIRONMENT = "release-signing";
/** Holds the public key that installed apps check updates against. */
export const TAURI_CONFIG = "apps/desktop/src-tauri/tauri.conf.json";

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const TRUSTED_COMMENT = "trusted comment: ";

/** Tauri stores minisign keys and signatures base64-encoded; each decodes to minisign's text lines. */
function minisignLines(text) {
  return Buffer.from(text.trim(), "base64").toString("utf8").split("\n");
}

function parsePublicKey(pubkey) {
  const [comment, line] = minisignLines(pubkey);
  const bytes = Buffer.from(line ?? "", "base64");
  if (!comment?.startsWith("untrusted comment:") || bytes.length !== 42 || bytes.subarray(0, 2).toString() !== "Ed") {
    throw new Error(
      `plugins.updater.pubkey in ${TAURI_CONFIG} is not a minisign public key. Set it to the contents of the .pub file that \`pnpm tauri signer generate\` wrote.`,
    );
  }
  const key = createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, bytes.subarray(10)]),
    format: "der",
    type: "spki",
  });
  return { id: bytes.subarray(2, 10), key };
}

/** Returns the updater's public key from tauri.conf.json, and refuses an empty or malformed one. */
export function updaterPubkey(root = ROOT) {
  const config = JSON.parse(readFileSync(join(root, TAURI_CONFIG), "utf8"));
  const pubkey = config.plugins?.updater?.pubkey ?? "";
  if (!pubkey) {
    throw new Error(
      `plugins.updater.pubkey in ${TAURI_CONFIG} is empty. Set it to the public key of the update signing key, as docs/releasing.md describes.`,
    );
  }
  parsePublicKey(pubkey);
  return pubkey;
}

/**
 * Checks a Tauri updater signature of `data` against `pubkey` as installed apps do, and returns the
 * version the signature was made for, or null when it names none.
 */
export function verifyUpdaterSignature(data, signature, pubkey) {
  const { id, key } = parsePublicKey(pubkey);
  const [, line, trustedLine, globalLine] = minisignLines(signature);
  const bytes = Buffer.from(line ?? "", "base64");
  if (bytes.length !== 74 || !trustedLine?.startsWith(TRUSTED_COMMENT))
    throw new Error("The updater signature is not a minisign signature.");
  if (!bytes.subarray(2, 10).equals(id)) {
    throw new Error(
      `The installer was signed with another key than the one in ${TAURI_CONFIG}, so installed apps would refuse the update. Check that TAURI_SIGNING_PRIVATE_KEY in the ${SIGNING_ENVIRONMENT} environment belongs to that public key.`,
    );
  }
  // "ED" signs the BLAKE2b-512 hash of the file, "Ed" the file itself.
  const message = bytes.subarray(0, 2).toString() === "ED" ? createHash("blake2b512").update(data).digest() : data;
  const signed = bytes.subarray(10);
  const trusted = trustedLine.slice(TRUSTED_COMMENT.length);
  const global = Buffer.from(globalLine ?? "", "base64");
  if (
    !verify(null, message, key, signed) ||
    !verify(null, Buffer.concat([signed, Buffer.from(trusted)]), key, global)
  ) {
    throw new Error("The updater signature does not match the installer.");
  }
  return (
    trusted
      .split("\t")
      .find((field) => field.startsWith("version:"))
      ?.slice("version:".length) ?? null
  );
}
