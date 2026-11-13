#!/usr/bin/env node
/**
 * Contrast check for the documentation code blocks.
 *
 * The defect this guards against: a dark code-block background in light mode
 * while Shiki emits dark light-theme tokens, which makes JSON keys and values
 * unreadable. The check reads the real token colours from the built HTML and
 * the real backgrounds from the theme CSS, then applies the WCAG contrast
 * formula.
 *
 * Normal text must reach 4.5:1. Code tokens are normal-size text, so the same
 * threshold applies.
 */
import fs from "node:fs";
import path from "node:path";

const root = process.cwd();
const MIN_CONTRAST = 4.5;

const PAGES = [
  "reference/profile-schema.html",
  "reference/cli.html",
  "guide/quick-start.html",
  "architecture/index.html",
  "security/index.html",
];

function readThemeBlock(selector) {
  const css = fs.readFileSync(path.join(root, ".vitepress/theme/custom.css"), "utf8");
  const pattern = new RegExp(`${selector}\\s*\\{([\\s\\S]*?)\\n\\}`, "m");
  const match = css.match(pattern);
  return match ? match[1] : "";
}

function cssVariable(block, name) {
  const match = block.match(new RegExp(`${name}\\s*:\\s*([^;]+);`));
  return match ? match[1].trim() : null;
}

function parseHex(value) {
  const match = /^#([0-9a-f]{6})$/i.exec(value.trim());
  if (!match) return null;
  const int = parseInt(match[1], 16);
  return [(int >> 16) & 255, (int >> 8) & 255, int & 255];
}

function relativeLuminance([r, g, b]) {
  const channel = (value) => {
    const c = value / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrastRatio(foreground, background) {
  const a = relativeLuminance(foreground);
  const b = relativeLuminance(background);
  const lighter = Math.max(a, b);
  const darker = Math.min(a, b);
  return (lighter + 0.05) / (darker + 0.05);
}

const lightBackground = parseHex(cssVariable(readThemeBlock(":root"), "--vp-code-block-bg") || "");
const darkBackground = parseHex(cssVariable(readThemeBlock("\\.dark"), "--vp-code-block-bg") || "");

if (!lightBackground || !darkBackground) {
  console.error("could not read --vp-code-block-bg for both themes");
  process.exit(1);
}

const failures = [];
let checked = 0;

for (const page of PAGES) {
  const file = path.join(root, ".vitepress/dist", page);
  if (!fs.existsSync(file)) {
    console.error(`missing built page: ${page}`);
    process.exit(1);
  }

  const html = fs.readFileSync(file, "utf8");

  for (const [mode, variable, background] of [
    ["light", "--shiki-light", lightBackground],
    ["dark", "--shiki-dark", darkBackground],
  ]) {
    const pattern = new RegExp(`${variable}:(#[0-9a-fA-F]{6})`, "g");
    const seen = new Set();
    let match;

    while ((match = pattern.exec(html)) !== null) {
      const hex = match[1].toLowerCase();
      if (seen.has(hex)) continue;
      seen.add(hex);

      const foreground = parseHex(hex);
      if (!foreground) continue;

      const ratio = contrastRatio(foreground, background);
      checked += 1;

      if (ratio < MIN_CONTRAST) {
        failures.push(
          `${page} [${mode}] ${hex} on ${variable === "--shiki-light" ? "#" + lightBackground.map((v) => v.toString(16).padStart(2, "0")).join("") : "#" + darkBackground.map((v) => v.toString(16).padStart(2, "0")).join("")} = ${ratio.toFixed(2)}:1`,
        );
      }
    }
  }
}

if (failures.length > 0) {
  console.error(`contrast check failed (${failures.length} token colours below ${MIN_CONTRAST}:1):`);
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}

console.log(`contrast check passed: ${checked} token colours at or above ${MIN_CONTRAST}:1`);
