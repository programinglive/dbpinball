#!/usr/bin/env node
/**
 * Internal link check for the built documentation site.
 *
 * Fails when a page links to a route that does not exist in the build output.
 * External links are not fetched: the check is offline and deterministic.
 */
import fs from "node:fs";
import path from "node:path";

const dist = path.join(process.cwd(), ".vitepress", "dist");

if (!fs.existsSync(dist)) {
  console.error("build output not found; run `npm run build` first");
  process.exit(1);
}

const pages = [];
(function walk(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const full = path.join(directory, entry.name);
    if (entry.isDirectory()) walk(full);
    else if (entry.name.endsWith(".html")) pages.push(full);
  }
})(dist);

function resolves(url) {
  const clean = url.split("#")[0].split("?")[0];
  if (!clean) return true;
  if (/^[a-z]+:/i.test(clean)) return true;

  let target = clean.startsWith("/") ? clean.slice(1) : clean;
  let file = path.join(dist, target);

  if (fs.existsSync(file) && fs.statSync(file).isDirectory()) {
    file = path.join(file, "index.html");
  }

  return fs.existsSync(file) || fs.existsSync(`${file}.html`);
}

let broken = 0;
let total = 0;

for (const page of pages) {
  const html = fs.readFileSync(page, "utf8");
  const pattern = /href="([^"]+)"/g;
  let match;

  while ((match = pattern.exec(html)) !== null) {
    const url = match[1];
    if (url.startsWith("#") || url.startsWith("mailto:") || url.startsWith("javascript:")) {
      continue;
    }
    total += 1;
    if (!resolves(url)) {
      broken += 1;
      console.error(`broken link: ${page.replace(dist, "")} -> ${url}`);
    }
  }
}

console.log(`checked ${total} links across ${pages.length} pages; broken: ${broken}`);
process.exit(broken === 0 ? 0 : 1);
