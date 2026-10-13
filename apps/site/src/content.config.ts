import { defineCollection } from "astro:content";
import { z } from "astro/zod";
import { gitRef, readDocs, workingTree, type DocsSource } from "./lib/docs";
import { loadRelease } from "./lib/release";

/**
 * The user docs: the built-in skill as of the release the site offers, so that they describe the
 * version people download. HATOBA_DOCS_REF names another git ref, or `worktree` for the files as
 * they are, to preview a change to the docs.
 */
const docs = defineCollection({
  loader: {
    name: "hatoba-docs",
    load: async ({ store, renderMarkdown, logger }) => {
      const wanted = process.env.HATOBA_DOCS_REF;
      let source: DocsSource;
      if (wanted === "worktree") {
        source = workingTree();
      } else {
        const ref = wanted || (await loadRelease()).tag;
        try {
          source = gitRef(ref);
        } catch {
          if (process.env.CI) throw new Error(`The docs come from ${ref}, which this checkout does not have. Check it out with its tags (fetch-depth: 0).`);
          logger.warn(`This checkout does not have ${ref}, so the docs come from the working tree. Run git fetch --tags to build them from ${ref}.`);
          source = workingTree();
        }
      }
      const version = source.ref?.replace(/^v/, "") ?? null;
      store.clear();
      for (const doc of readDocs(source, version ?? "development")) {
        const { slug, body, ...data } = doc;
        store.set({ id: slug, data: { ...data, version }, body, rendered: await renderMarkdown(body) });
      }
    },
  },
  schema: z.object({
    title: z.string(),
    description: z.string(),
    editUrl: z.string(),
    /** The release the docs describe, or null when they come from the working tree. */
    version: z.string().nullable(),
  }),
});

export const collections = { docs };
