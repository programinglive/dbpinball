#!/usr/bin/env node

/**
 * Dependency boundary check for dbpinball-core.
 *
 * The core crate must stay headless: no Tauri, no CLI parser, no terminal I/O.
 * This script fails the build when either leak appears in the manifest or in
 * the resolved dependency graph.
 */

const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const root = path.resolve(__dirname, '..');
const manifestPath = path.join(root, 'crates', 'dbpinball-core', 'Cargo.toml');
const forbidden = ['tauri', 'clap'];

function fail(message) {
  console.error(`boundary check failed: ${message}`);
  process.exit(1);
}

if (!fs.existsSync(manifestPath)) {
  fail(`manifest not found at ${manifestPath}`);
}

const manifest = fs.readFileSync(manifestPath, 'utf8');

for (const name of forbidden) {
  const pattern = new RegExp(`^\\s*${name}\\s*=`, 'm');
  if (pattern.test(manifest)) {
    fail(`crates/dbpinball-core/Cargo.toml declares a dependency on "${name}"`);
  }
}

let tree;

try {
  tree = execFileSync('cargo', ['tree', '-p', 'dbpinball-core', '--prefix', 'none'], {
    cwd: root,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  });
} catch (error) {
  fail(`cargo tree failed: ${error.message}`);
}

const leaks = tree
  .split('\n')
  .map((line) => line.trim())
  .filter((line) => line.length > 0)
  .filter((line) => forbidden.some((name) => line === name || line.startsWith(`${name} v`)));

if (leaks.length > 0) {
  fail(`resolved graph contains a forbidden dependency:\n  ${leaks.join('\n  ')}`);
}

console.log(`boundary check passed: dbpinball-core depends on neither tauri nor clap`);
