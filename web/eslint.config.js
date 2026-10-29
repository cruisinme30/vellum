// SPDX-FileCopyrightText: 2026 The Vellum contributors
// SPDX-License-Identifier: AGPL-3.0-only

import js from "@eslint/js";
import { defineConfig } from "eslint/config";
import prettier from "eslint-config-prettier";
import { createTypeScriptImportResolver } from "eslint-import-resolver-typescript";
import { flatConfigs as importX } from "eslint-plugin-import-x";
import jsxA11y from "eslint-plugin-jsx-a11y";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import { configs as sonarjs } from "eslint-plugin-sonarjs";
import unicorn from "eslint-plugin-unicorn";
import globals from "globals";
import { configs as tseslint } from "typescript-eslint";

export default defineConfig(
  { ignores: ["dist", "coverage", "playwright-report", "test-results"] },
  js.configs.recommended,
  tseslint.strictTypeChecked,
  tseslint.stylisticTypeChecked,
  react.configs.flat.recommended,
  react.configs.flat["jsx-runtime"],
  reactHooks.configs.flat["recommended-latest"],
  jsxA11y.flatConfigs.strict,
  importX.recommended,
  importX.typescript,
  unicorn.configs.recommended,
  sonarjs.recommended,
  {
    languageOptions: {
      globals: { ...globals.browser },
      parserOptions: {
        projectService: { allowDefaultProject: ["eslint.config.js"] },
        tsconfigRootDir: import.meta.dirname,
      },
    },
    settings: {
      react: { version: "detect" },
      "import-x/resolver-next": [createTypeScriptImportResolver()],
    },
    rules: {
      "max-lines": ["error", { max: 400, skipBlankLines: true, skipComments: true }],
      complexity: ["error", 12],
      "import-x/no-default-export": "error",
      "import-x/order": ["error", { "newlines-between": "always", alphabetize: { order: "asc" } }],
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-non-null-assertion": "error",
      "no-restricted-syntax": [
        "error",
        { selector: "TSAnyKeyword", message: "Use unknown and narrow it." },
        { selector: "TSNonNullExpression", message: "Handle the missing case instead of `!`." },
      ],
    },
  },
  {
    // This file wires untyped plugins together; lint it without type info.
    files: ["eslint.config.js"],
    extends: [tseslint.disableTypeChecked],
  },
  {
    // Tool configuration files must use a default export.
    files: ["*.config.{js,ts}"],
    languageOptions: { globals: { ...globals.node } },
    rules: { "import-x/no-default-export": "off" },
  },
  prettier,
);
