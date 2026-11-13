#!/usr/bin/env node
/**
 * Layout regression check for the documentation site.
 *
 * The defect this guards against: a `.vp-doc table` rule that forces
 * `display: table` makes a wide table expand to its content width, which pushes
 * the whole document sideways on a 390px viewport. VitePress ships
 * `display: block; overflow-x: auto` precisely so a wide table scrolls inside
 * its own box instead.
 *
 * The check reads the built stylesheet, so a future theme override cannot
 * silently reintroduce the regression.
 */
import fs from "node:fs";
import path from "node:path";

const dist = path.join(process.cwd(), ".vitepress", "dist");
const assets = path.join(dist, "assets");

if (!fs.existsSync(assets)) {
  console.error("build output not found; run `npm run build` first");
  process.exit(1);
}

const stylesheets = fs
  .readdirSync(assets)
  .filter((name) => name.startsWith("style.") && name.endsWith(".css"));

if (stylesheets.length === 0) {
  console.error("no built stylesheet found");
  process.exit(1);
}

const css = stylesheets
  .map((name) => fs.readFileSync(path.join(assets, name), "utf8"))
  .join("\n");

function allRulesFor(selector) {
  const rules = [];
  let index = 0;

  while ((index = css.indexOf(selector, index)) >= 0) {
    const open = css.indexOf("{", index);
    if (open < 0) break;
    const close = css.indexOf("}", open);
    if (close < 0) break;
    rules.push(css.slice(open + 1, close));
    index = close + 1;
  }

  return rules;
}

const tableRules = allRulesFor(".vp-doc table");
const codeRules = allRulesFor(".vp-doc div[class*=");
const failures = [];

if (tableRules.length === 0) {
  failures.push("no .vp-doc table rule found in the built stylesheet");
}

// A competing `display: table` wins or loses by cascade order, so any
// occurrence is treated as a regression rather than guessing the order.
for (const rule of tableRules) {
  if (/display\s*:\s*table/.test(rule)) {
    failures.push(
      ".vp-doc table forces `display: table`, which defeats the scroll wrapper and overflows mobile",
    );
  }
}

if (!tableRules.some((rule) => /overflow-x\s*:\s*auto/.test(rule))) {
  failures.push(".vp-doc table lost `overflow-x: auto`; a wide table will overflow the page");
}

// The code block must keep its own horizontal scroll for long lines.
if (codeRules.length > 0 && !codeRules.some((rule) => /overflow-x\s*:\s*auto/.test(rule))) {
  failures.push(".vp-doc code block lost `overflow-x: auto`");
}

if (failures.length > 0) {
  console.error("layout check failed:");
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}

console.log("layout check passed: tables and code blocks keep their scroll wrappers");
