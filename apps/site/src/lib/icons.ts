import { readFileSync } from "node:fs";
import { join } from "node:path";
import sharp from "sharp";
import { repoRoot } from "./docs";

/** The app's icon, which the site serves at fixed addresses for browsers and search engines. */
export function appIcon(file = "app-icon.svg"): Buffer {
  return readFileSync(join(repoRoot(), "apps/desktop/src-tauri", file));
}

/** The app icon as a square PNG of `size` pixels, on `background` when given. */
export async function appIconPng(size: number, background?: string): Promise<Uint8Array<ArrayBuffer>> {
  let image = sharp(appIcon()).resize(size, size);
  if (background) image = image.flatten({ background });
  return new Uint8Array(await image.png().toBuffer());
}
