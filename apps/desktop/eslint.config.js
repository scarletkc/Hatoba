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
      // These React Compiler rules report patterns the app uses on purpose: reading and writing refs
      // during render, Date.now() during render, setting state in an effect, and module variables
      // that remember UI state across mounts.
      "react-hooks/refs": "off",
      "react-hooks/purity": "off",
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/globals": "off",
    },
  },
);
