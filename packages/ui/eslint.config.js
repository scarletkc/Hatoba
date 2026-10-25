import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default defineConfig({
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
    // These React Compiler rules report patterns the components use on purpose: reading and writing
    // refs during render, and setting state in an effect.
    "react-hooks/refs": "off",
    "react-hooks/set-state-in-effect": "off",
  },
});
