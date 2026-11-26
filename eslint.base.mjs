import js from "@eslint/js";
import { globalIgnores } from "eslint/config";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export function baseConfig(tsconfigRootDir, { ignores = [], rules = {} } = {}) {
  return [
    globalIgnores(["coverage", ...ignores]),
    {
      files: ["**/*.{ts,tsx}"],
      extends: [js.configs.recommended, tseslint.configs.recommended, reactHooks.configs.flat.recommended],
      languageOptions: {
        parserOptions: { projectService: true, tsconfigRootDir },
      },
      rules: {
        "@typescript-eslint/no-floating-promises": "error",
        "@typescript-eslint/no-misused-promises": "error",
        // tsc reports unused locals and parameters (noUnusedLocals and noUnusedParameters in tsconfig.base.json).
        "@typescript-eslint/no-unused-vars": "off",
        // These React Compiler rules report patterns the app and UI components use on purpose:
        // reading and writing refs during render and setting state in an effect.
        "react-hooks/refs": "off",
        "react-hooks/set-state-in-effect": "off",
        ...rules,
      },
    },
  ];
}
