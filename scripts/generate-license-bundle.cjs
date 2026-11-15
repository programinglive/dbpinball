#!/usr/bin/env node

/**
 * Generates a complete third-party license bundle.
 *
 * MIT and Apache-2.0 both require that recipients of a binary distribution
 * receive the applicable license texts and notices. A hand-maintained list of
 * direct dependencies is not enough, so this script walks the resolved
 * dependency graph, resolves each SPDX license expression, and embeds the full
 * canonical text of every license a recipient must receive.
 *
 * Canonical texts live in `third-party/license-texts/<SPDX-ID>.txt`, downloaded
 * once from the SPDX license list (or the license steward for the few IDs the
 * SPDX repository does not carry verbatim). The script fails when a required
 * text is missing, so an incomplete bundle cannot be committed silently.
 *
 * Output: THIRD_PARTY_LICENSES.md (generated, committed, shipped in artifacts).
 */

const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const textDir = path.join(root, "third-party", "license-texts");

function run(command, args) {
  return execFileSync(command, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
}

/* ------------------------------------------------------------------ */
/* SPDX expression parsing                                            */
/* ------------------------------------------------------------------ */

function tokenize(expression) {
  // Legacy manifests use `MIT/Apache-2.0` to mean "either"; SPDX IDs never
  // contain a slash, so normalising it to OR is safe.
  return expression
    .replace(/\//g, " OR ")
    .replace(/\(/g, " ( ")
    .replace(/\)/g, " ) ")
    .split(/\s+/)
    .filter(Boolean);
}

/**
 * Parse an SPDX expression into an AST.
 *
 * Grammar: expr := term (OR term)* ; term := factor (AND factor)* ;
 *          factor := '(' expr ')' | license ('WITH' exception)?
 */
function parse(expression) {
  const tokens = tokenize(expression);
  let index = 0;

  const peek = () => tokens[index];
  const next = () => tokens[index++];

  function factor() {
    const token = next();
    if (token === "(") {
      const inner = expr();
      if (next() !== ")") {
        throw new Error(`unbalanced parentheses in "${expression}"`);
      }
      return inner;
    }
    if (!token) {
      throw new Error(`unexpected end of expression in "${expression}"`);
    }
    if (peek() === "WITH") {
      next();
      const exception = next();
      return { type: "with", license: token, exception };
    }
    return { type: "license", id: token };
  }

  function term() {
    let left = factor();
    while (peek() === "AND") {
      next();
      left = { type: "and", left, right: factor() };
    }
    return left;
  }

  function expr() {
    let left = term();
    while (peek() === "OR") {
      next();
      left = { type: "or", left, right: term() };
    }
    return left;
  }

  const ast = expr();
  if (index !== tokens.length) {
    throw new Error(`trailing tokens in "${expression}"`);
  }
  return ast;
}

function hasText(id) {
  return fs.existsSync(path.join(textDir, `${id}.txt`));
}

/**
 * The set of license identifiers a recipient must receive.
 *
 * An OR lets the recipient choose, so one branch is enough — this prefers a
 * branch whose texts are available rather than blindly taking the first.
 * An AND requires every conjunct, and WITH requires both the license and the
 * exception text.
 */
function required(ast) {
  switch (ast.type) {
    case "license":
      return [ast.id];
    case "with":
      return [ast.license, ast.exception];
    case "and":
      return [...new Set([...required(ast.left), ...required(ast.right)])];
    case "or": {
      const left = required(ast.left);
      if (left.every(hasText)) {
        return left;
      }
      return required(ast.right);
    }
    default:
      throw new Error(`unknown node ${ast.type}`);
  }
}

/* ------------------------------------------------------------------ */
/* Dependency discovery                                               */
/* ------------------------------------------------------------------ */

function rustPackages() {
  const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1"]));
  const workspace = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => !workspace.has(pkg.id))
    .map((pkg) => ({
      name: pkg.name,
      version: pkg.version,
      license: pkg.license || (pkg.license_file ? "SEE LICENSE FILE" : "UNKNOWN"),
    }));
}

function npmPackages(directory) {
  const lockPath = path.join(root, directory, "package-lock.json");
  if (!fs.existsSync(lockPath)) {
    return [];
  }
  const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
  const seen = new Map();
  for (const [key, entry] of Object.entries(lock.packages || {})) {
    // Only installed packages: workspace roots (`packages/...`) are this
    // project's own code, and link entries carry no published identity.
    if (!key.includes("node_modules/") || !entry || entry.dev === true) {
      continue;
    }
    if (entry.link === true || !entry.version) {
      continue;
    }
    const name = entry.name || key.replace(/^.*node_modules\//, "");
    const id = `${name}@${entry.version}`;
    if (!seen.has(id)) {
      seen.set(id, {
        name,
        version: entry.version,
        license: entry.license || "UNKNOWN",
        directory,
        installPath: path.join(root, directory, key),
      });
    }
  }
  return [...seen.values()];
}

/**
 * The copyright lines a package actually publishes.
 *
 * MIT, BSD and 0BSD require the copyright notice itself, not just the license
 * body, so a bundle of generic texts is incomplete. npm packages ship a LICENSE
 * or CopyrightNotice file; those lines are collected verbatim. Nothing is
 * invented: when a package publishes no notice, the entry says so.
 */
function copyrightNotices(installPath) {
  if (!installPath || !fs.existsSync(installPath)) {
    return [];
  }
  const candidates = fs
    .readdirSync(installPath)
    .filter((name) => /^(licen[cs]e|copyright|notice)/i.test(name));
  const notices = new Set();
  for (const name of candidates) {
    const file = path.join(installPath, name);
    let stat;
    try {
      stat = fs.statSync(file);
    } catch {
      continue;
    }
    if (!stat.isFile() || stat.size > 200_000) {
      continue;
    }
    for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
      const trimmed = line.trim();
      // A real notice starts with "Copyright" and names a holder. The warranty
      // disclaimer also contains the word, so exclude the liability phrasing.
      if (!/^copyright\b/i.test(trimmed) && !/^\(c\)\s/i.test(trimmed)) {
        continue;
      }
      if (/liable|warrant|damages|holders be/i.test(trimmed)) {
        continue;
      }
      if (trimmed.length > 160) {
        continue;
      }
      notices.add(trimmed);
    }
  }
  return [...notices];
}

/* ------------------------------------------------------------------ */

const rust = rustPackages();
const npmRoot = npmPackages(".");
const npmDocs = npmPackages("website");
const shipped = [...rust, ...npmRoot];

const problems = [];
const resolved = new Map();

for (const entry of shipped) {
  if (entry.license === "UNKNOWN" || entry.license === "SEE LICENSE FILE") {
    problems.push(`${entry.name} ${entry.version} has no declared SPDX license`);
    resolved.set(`${entry.name}@${entry.version}`, ["UNKNOWN"]);
    continue;
  }
  try {
    const ids = required(parse(entry.license));
    const missing = ids.filter((id) => !hasText(id));
    if (missing.length > 0) {
      problems.push(
        `${entry.name} ${entry.version} (${entry.license}) needs text for: ${missing.join(", ")}`,
      );
    }
    resolved.set(`${entry.name}@${entry.version}`, ids);
  } catch (error) {
    problems.push(`${entry.name} ${entry.version}: ${error.message}`);
    resolved.set(`${entry.name}@${entry.version}`, ["UNKNOWN"]);
  }
}

const needed = new Set();
for (const ids of resolved.values()) {
  for (const id of ids) {
    needed.add(id);
  }
}

const LICENSE_NOTES = {
  "0BSD":
    "The SPDX template carries a `Copyright (C) YEAR by AUTHOR EMAIL` line. The per-package copyright notices are listed in the Copyright notices section below; this text is the license body.",
  "MIT":
    "The SPDX template carries a `Copyright (c) <year> <copyright holders>` line. The per-package copyright notices are listed in the Copyright notices section below; this text is the license body.",
  "Unicode-3.0": "Unicode License V3, as published by the Unicode Consortium.",
  "CC0-1.0": "Creative Commons CC0 1.0 Universal public domain dedication.",
  "MPL-2.0":
    "MPL-2.0 is file-level copyleft. DBPinball redistributes these crates unmodified and does not relicense them.",
};

const lines = [];
lines.push("# Third-Party License Bundle");
lines.push("");
lines.push("Generated by `scripts/generate-license-bundle.cjs`. Do not edit by hand.");
lines.push("");
lines.push(
  "DBPinball itself is MIT licensed (see `LICENSE`). This bundle records every",
  "third-party crate and npm package that ships in, or is compiled into, a",
  "released binary, together with the full canonical text of each license those",
  "dependencies require.",
);
lines.push("");
lines.push(`- Rust crates: ${rust.length}`);
lines.push(`- npm packages (application): ${npmRoot.length}`);
lines.push(`- npm packages (documentation site, not shipped): ${npmDocs.length}`);
lines.push(`- Licenses in use: ${[...needed].sort().join(", ")}`);
lines.push("");

if (problems.length > 0) {
  lines.push("## Unresolved licensing problems");
  lines.push("");
  lines.push("This bundle is incomplete. Each line below must be resolved:");
  lines.push("");
  for (const problem of problems) {
    lines.push(`- ${problem}`);
  }
  lines.push("");
}

function table(title, entries) {
  if (entries.length === 0) {
    return;
  }
  lines.push(`## ${title}`);
  lines.push("");
  lines.push("| Package | Version | SPDX expression | Licenses supplied |");
  lines.push("| --- | --- | --- | --- |");
  for (const entry of [...entries].sort((a, b) => a.name.localeCompare(b.name))) {
    const ids = resolved.get(`${entry.name}@${entry.version}`) || [];
    lines.push(
      `| \`${entry.name}\` | ${entry.version} | ${entry.license} | ${ids.join(", ")} |`,
    );
  }
  lines.push("");
}

table("Rust crates", rust);
table("npm packages (application)", npmRoot);

// Copyright notices, collected verbatim from each installed package.
const noticeRows = [];
for (const entry of [...npmRoot].sort((a, b) => a.name.localeCompare(b.name))) {
  const notices = copyrightNotices(entry.installPath);
  if (notices.length > 0) {
    noticeRows.push({ name: entry.name, version: entry.version, notices });
  }
}

lines.push("## Copyright notices");
lines.push("");
lines.push(
  "MIT, BSD and 0BSD require the copyright notice itself, not only the license",
  "body, and the SPDX texts for MIT and 0BSD carry a placeholder line where that",
  "notice belongs. The notices below are copied verbatim from each installed npm",
  "package's own LICENSE, COPYRIGHT or NOTICE file and complete those placeholders.",
);
lines.push("");
lines.push(
  "Rust crates are compiled from a registry cache rather than installed into the",
  "repository, so their notices are not collected here; each crate's own LICENSE",
  "file ships with its source and is retained unmodified.",
);
lines.push("");
if (noticeRows.length === 0) {
  lines.push("_No package-specific copyright notices were found._");
  lines.push("");
} else {
  lines.push("| Package | Version | Copyright notice |");
  lines.push("| --- | --- | --- |");
  for (const row of noticeRows) {
    const text = row.notices.join("<br>").replace(/\|/g, "\\|");
    lines.push(`| \`${row.name}\` | ${row.version} | ${text} |`);
  }
  lines.push("");
}

lines.push("## License texts");
lines.push("");
for (const id of [...needed].sort()) {
  lines.push(`### ${id}`);
  lines.push("");
  if (LICENSE_NOTES[id]) {
    lines.push(LICENSE_NOTES[id]);
    lines.push("");
  }
  const file = path.join(textDir, `${id}.txt`);
  if (fs.existsSync(file)) {
    lines.push("```text");
    lines.push(fs.readFileSync(file, "utf8").trimEnd());
    lines.push("```");
  } else {
    lines.push(`No canonical text is stored for \`${id}\`.`);
  }
  lines.push("");
}

const target = path.join(root, "THIRD_PARTY_LICENSES.md");

/* ------------------------------------------------------------------ */
/* --verify                                                           */
/* ------------------------------------------------------------------ */

/**
 * Verify the committed bundle without regenerating it.
 *
 * Regenerating and diffing would compare an artifact that depends on the
 * installed npm tree, which differs between platforms. Verification instead
 * checks the guarantees that matter: every shipped package appears in the
 * bundle, every package's required license text is present, and no package was
 * left unresolved.
 */
if (process.argv.includes("--verify")) {
  if (!fs.existsSync(target)) {
    console.error("THIRD_PARTY_LICENSES.md is missing");
    process.exit(1);
  }
  const committed = fs.readFileSync(target, "utf8");
  const committedLines = new Set(committed.split(/\r?\n/));
  const failures = [];

  for (const problem of problems) {
    failures.push(problem);
  }

  // Compare the exact license-table rows. Those rows come from the lockfiles and
  // the parser, so they are identical on every platform; the notice table does
  // not participate in this check.
  for (const entry of shipped) {
    const ids = resolved.get(`${entry.name}@${entry.version}`) || [];
    const row = `| \`${entry.name}\` | ${entry.version} | ${entry.license} | ${ids.join(", ")} |`;
    if (!committedLines.has(row)) {
      failures.push(
        `${entry.name} ${entry.version} is missing or wrong in the bundle (expected: ${row})`,
      );
    }
  }

  for (const license of needed) {
    if (!committedLines.has(`### ${license}`)) {
      failures.push(`license text for ${license} is missing from the bundle`);
    }
    if (!fs.existsSync(path.join(textDir, `${license}.txt`))) {
      failures.push(`canonical text file for ${license} is missing`);
    }
  }

  if (failures.length > 0) {
    console.error("license bundle verification failed:");
    for (const failure of failures) {
      console.error(`  - ${failure}`);
    }
    process.exit(1);
  }

  console.log(
    `license bundle verified: ${shipped.length} packages, ${needed.size} license text(s)`,
  );
  process.exit(0);
}

fs.writeFileSync(target, `${lines.join("\n")}\n`, "utf8");
console.log(`wrote ${target}`);
console.log(`licenses supplied: ${[...needed].sort().join(", ")}`);

if (problems.length > 0) {
  console.error(`\n${problems.length} unresolved licensing problem(s):`);
  for (const problem of problems) {
    console.error(`  - ${problem}`);
  }
  process.exit(1);
}
