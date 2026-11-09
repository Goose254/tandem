// Sets the app version in the four files that carry it (see docs/RELEASING.md).
//
//   node scripts/set-version.mjs 1.2.3
//
// Afterwards refresh the lock files: `npm install --package-lock-only` and `cargo check`.

import { readFileSync, writeFileSync } from "node:fs";

const next = process.argv[2];
if (!/^\d+\.\d+\.\d+$/.test(next ?? "")) {
  console.error("Usage: node scripts/set-version.mjs <major.minor.patch>");
  process.exit(1);
}

/** Replace the first match of `pattern` in `file`, keeping the file's line endings. */
function replaceOnce(file, pattern, replacement) {
  const text = readFileSync(file, "utf8");
  if (!pattern.test(text)) {
    console.error(`No version found in ${file}`);
    process.exit(1);
  }
  writeFileSync(file, text.replace(pattern, replacement));
  console.log(`${file}: ${next}`);
}

replaceOnce("package.json", /("version":\s*")\d+\.\d+\.\d+(")/, `$1${next}$2`);
replaceOnce("src-tauri/tauri.conf.json", /("version":\s*")\d+\.\d+\.\d+(")/, `$1${next}$2`);
// The first `version = "..."` in a Cargo manifest is the package's own.
replaceOnce("src-tauri/Cargo.toml", /(^version\s*=\s*")\d+\.\d+\.\d+(")/m, `$1${next}$2`);
replaceOnce("engine/Cargo.toml", /(^version\s*=\s*")\d+\.\d+\.\d+(")/m, `$1${next}$2`);
