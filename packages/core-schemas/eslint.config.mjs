import { config as baseConfig } from "@repo/eslint-config/base";

export default [
  ...baseConfig,
  {
    languageOptions: {
      parserOptions: {
        project: "./tsconfig.json",
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },
  {
    ignores: ["eslint.config.mjs", "vitest.config.ts"],
  },
]; 