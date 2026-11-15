#!/usr/bin/env node

/**
 * Version consistency check.
 *
 * The product version appears in several manifests. A release where the
 * installer, the CLI and the UI disagree is a defect users can see, so this
 * script fails the build when they drift.
 *
 * `crates/dbpinball-core` is an internal library with its own version and is
 * deliberately not part of the product version.
 */

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");

function readJson(relative) {
  return JSON.parse(fs.readFileSync(path.join(root, relative), "utf8"));
}

function readCargoVersion(relative) {
  const text = fs.readFileSync(path.join(root, relative), "utf8");
  const match = text.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!match) {
    throw new Error(`no version field in ${relative}`);
  }
  return match[1];
}

const sources = [
  ["package.json", readJson("package.json").version],
  ["src-tauri/tauri.conf.json", readJson("src-tauri/tauri.conf.json").version],
  ["crates/dbpinball-cli/Cargo.toml", readCargoVersion("crates/dbpinball-cli/Cargo.toml")],
  ["src-tauri/Cargo.toml", readCargoVersion("src-tauri/Cargo.toml")],
  ["website/package.json", readJson("website/package.json").version],
];

const expected = sources[0][1];
const mismatched = sources.filter(([, version]) => version !== expected);

for (const [file, version] of sources) {
  console.log(`${version === expected ? "ok  " : "FAIL"} ${file}: ${version}`);
}

if (mismatched.length > 0) {
  console.error(
    `\nversion mismatch: expected ${expected} everywhere, found ${mismatched
      .map(([file, version]) => `${file}=${version}`)
      .join(", ")}`,
  );
  process.exit(1);
}

console.log(`\nversion check passed: ${expected}`);
