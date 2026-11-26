import { defineConfig } from "eslint/config";
import { baseConfig } from "../../eslint.base.mjs";

export default defineConfig(
  ...baseConfig(import.meta.dirname, {
    ignores: ["dist", "e2e", "src-tauri", "src/ipc/bindings.ts"],
    rules: {
      // The app matches terminal escape sequences and control characters on purpose.
      "no-control-regex": "off",
      // These React Compiler rules report patterns the app uses on purpose: Date.now() during
      // render and module variables that remember UI state across mounts.
      "react-hooks/purity": "off",
      "react-hooks/globals": "off",
    },
  }),
  {
    // Shared layers never import a feature (#346); visitorStart.ts drives the AI panel UI in --mode demo.
    files: [
      "src/session/**/*.{ts,tsx}",
      "src/lib/**/*.{ts,tsx}",
      "src/ipc/**/*.{ts,tsx}",
      "src/i18n/**/*.{ts,tsx}",
      "src/app/store.ts",
    ],
    ignores: ["src/ipc/mock/visitorStart.ts"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              regex: "^(?:@/|(?:\\.\\./)+)features(?:/|$)",
              message: "Shared layers never import a feature (#346).",
            },
          ],
        },
      ],
      "no-restricted-syntax": [
        "error",
        {
          selector: "ImportExpression[source.value=/^(@\\/|(\\.\\.\\/)+)features(\\/|$)/]",
          message: "Shared layers never import a feature (#346).",
        },
      ],
    },
  },
);
