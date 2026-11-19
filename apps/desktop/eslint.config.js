import js from "@eslint/js";
import { defineConfig, globalIgnores } from "eslint/config";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default defineConfig(
  globalIgnores(["dist", "e2e", "src-tauri", "src/ipc/bindings.ts"]),
  {
    files: ["**/*.{ts,tsx}"],
    extends: [js.configs.recommended, tseslint.configs.recommended, reactHooks.configs.flat.recommended],
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
    },
    rules: {
      "@typescript-eslint/no-floating-promises": "error",
      "@typescript-eslint/no-misused-promises": "error",
      // tsc reports unused locals and parameters (noUnusedLocals and noUnusedParameters in tsconfig.json).
      "@typescript-eslint/no-unused-vars": "off",
      // The app matches terminal escape sequences and control characters on purpose.
      "no-control-regex": "off",
      // These React Compiler rules report patterns the app uses on purpose: reading and writing refs
      // during render, Date.now() during render, setting state in an effect, and module variables
      // that remember UI state across mounts.
      "react-hooks/refs": "off",
      "react-hooks/purity": "off",
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/globals": "off",
    },
  },
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
