// ESLint for the UI (src/) and the Node scripts. Type-aware rules are left off: `tsc` already
// type-checks the UI in `npm run build`, and this keeps `npm run lint` quick.
import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  // Build output and the Rust side are not linted.
  { ignores: ["dist/", "dist-demo/", "target/", "src-tauri/", "engine/"] },
  js.configs.recommended,
  tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [reactHooks.configs.flat.recommended],
    languageOptions: { globals: globals.browser },
  },
  {
    files: ["scripts/**/*.mjs", "*.config.{js,ts}"],
    languageOptions: { globals: globals.node },
  },
);
