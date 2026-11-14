import { fileURLToPath } from "node:url";
import * as SkillFilesystem from "@deepseek-ai/dsh-skill-filesystem";

const SKILLS_DIR = fileURLToPath(new URL("../skills", import.meta.url));

export const name = "hatoba";
export const inject = ["skills"];

export function apply(ctx) {
  ctx.plugin(SkillFilesystem, {
    providerName: "hatoba",
    includeDefaultRoots: false,
    customSkillDirs: [SKILLS_DIR],
  });
}
