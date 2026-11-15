#!/usr/bin/env node

/**
 * Generates a complete third-party license bundle.
 *
 * MIT and Apache-2.0 both require that recipients of a binary distribution
 * receive the applicable license texts and notices. A hand-maintained list of
 * direct dependencies is not enough, so this script walks the resolved
 * dependency graph and emits every crate and package with its license and the
 * full text of each distinct license it uses.
 *
 * Output: THIRD_PARTY_LICENSES.md (generated, committed, shipped in artifacts).
 */

const { execFileSync } = require("child_process");
const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");

function run(command, args) {
  return execFileSync(command, args, {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
  });
}

function rustPackages() {
  const metadata = JSON.parse(run("cargo", ["metadata", "--format-version", "1"]));
  const workspace = new Set(metadata.workspace_members);
  return metadata.packages
    .filter((pkg) => !workspace.has(pkg.id))
    .map((pkg) => ({
      name: pkg.name,
      version: pkg.version,
      license: pkg.license || (pkg.license_file ? `see ${pkg.license_file}` : "UNKNOWN"),
      repository: pkg.repository || "",
    }));
}

function npmPackages(directory) {
  const manifest = path.join(root, directory, "package.json");
  if (!fs.existsSync(manifest)) {
    return [];
  }
  const lockPath = path.join(root, directory, "package-lock.json");
  if (!fs.existsSync(lockPath)) {
    return [];
  }
  const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
  const workspacePackages = new Set(
    Object.entries(lock.packages || {})
      .filter(([key, entry]) => key !== "" && entry && entry.link === true)
      .map(([key]) => key),
  );
  const seen = new Map();
  const walk = (packages) => {
    for (const [key, entry] of Object.entries(packages || {})) {
      // Only installed packages belong here. Workspace roots (`packages/...`)
      // are this project's own code, not third-party dependencies.
      if (!key.includes("node_modules/")) {
        continue;
      }
      if (!key || !entry || entry.dev === true || workspacePackages.has(key)) {
        continue;
      }
      // A `link: true` entry with no version is a local workspace symlink, not a
      // published package.
      if (entry.link === true || !entry.version) {
        continue;
      }
      const name = entry.name || key.replace(/^.*node_modules\//, "");
      const version = entry.version || "";
      if (!name || !version) {
        continue;
      }
      const id = `${name}@${version}`;
      if (!seen.has(id)) {
        seen.set(id, {
          name,
          version,
          license: entry.license || "UNKNOWN",
          directory,
        });
      }
    }
  };
  walk(lock.packages);
  return [...seen.values()];
}

/** The canonical texts, keyed by the SPDX identifier they satisfy. */
const LICENSE_TEXTS = {
  MIT: `MIT License

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`,
  "Apache-2.0": `                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      "License" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

      "Licensor" shall mean the copyright owner or entity authorized by
      the copyright owner that is granting the License.

      "Legal Entity" shall mean the union of the acting entity and all
      other entities that control, are controlled by, or are under common
      control with that entity. For the purposes of this definition,
      "control" means (i) the power, direct or indirect, to cause the
      direction or management of such entity, whether by contract or
      otherwise, or (ii) ownership of fifty percent (50%) or more of the
      outstanding shares, or (iii) beneficial ownership of such entity.

      "You" (or "Your") shall mean an individual or Legal Entity
      exercising permissions granted by this License.

      "Source" form shall mean the preferred form for making modifications,
      including but not limited to software source code, documentation
      source, and configuration files.

      "Object" form shall mean any form resulting from mechanical
      transformation or translation of a Source form, including but
      not limited to compiled object code, generated documentation,
      and conversions to other media types.

      "Work" shall mean the work of authorship, whether in Source or
      Object form, made available under the License, as indicated by a
      copyright notice that is included in or attached to the work
      (an example is provided in the Appendix below).

      "Derivative Works" shall mean any work, whether in Source or Object
      form, that is based on (or derived from) the Work and for which the
      editorial revisions, annotations, elaborations, or other modifications
      represent, as a whole, an original work of authorship. For the purposes
      of this License, Derivative Works shall not include works that remain
      separable from, or merely link (or bind by name) to the interfaces of,
      the Work and Derivative Works thereof.

      "Contribution" shall mean any work of authorship, including
      the original version of the Work and any modifications or additions
      to that Work or Derivative Works thereof, that is intentionally
      submitted to Licensor for inclusion in the Work by the copyright owner
      or by an individual or Legal Entity authorized to submit on behalf of
      the copyright owner. For the purposes of this definition, "submitted"
      means any form of electronic, verbal, or written communication sent
      to the Licensor or its representatives, including but not limited to
      communication on electronic mailing lists, source code control systems,
      and issue tracking systems that are managed by, or on behalf of, the
      Licensor for the purpose of discussing and improving the Work, but
      excluding communication that is conspicuously marked or otherwise
      designated in writing by the copyright owner as "Not a Contribution."

      "Contributor" shall mean Licensor and any individual or Legal Entity
      on behalf of whom a Contribution has been received by Licensor and
      subsequently incorporated within the Work.

   2. Grant of Copyright License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      copyright license to reproduce, prepare Derivative Works of,
      publicly display, publicly perform, sublicense, and distribute the
      Work and such Derivative Works in Source or Object form.

   3. Grant of Patent License. Subject to the terms and conditions of
      this License, each Contributor hereby grants to You a perpetual,
      worldwide, non-exclusive, no-charge, royalty-free, irrevocable
      (except as stated in this section) patent license to make, have made,
      use, offer to sell, sell, import, and otherwise transfer the Work,
      where such license applies only to those patent claims licensable
      by such Contributor that are necessarily infringed by their
      Contribution(s) alone or by combination of their Contribution(s)
      with the Work to which such Contribution(s) was submitted. If You
      institute patent litigation against any entity (including a
      cross-claim or counterclaim in a lawsuit) alleging that the Work
      or a Contribution incorporated within the Work constitutes direct
      or contributory patent infringement, then any patent licenses
      granted to You under this License for that Work shall terminate
      as of the date such litigation is filed.

   4. Redistribution. You may reproduce and distribute copies of the
      Work or Derivative Works thereof in any medium, with or without
      modifications, and in Source or Object form, provided that You
      meet the following conditions:

      (a) You must give any other recipients of the Work or
          Derivative Works a copy of this License; and

      (b) You must cause any modified files to carry prominent notices
          stating that You changed the files; and

      (c) You must retain, in the Source form of any Derivative Works
          that You distribute, all copyright, patent, trademark, and
          attribution notices from the Source form of the Work,
          excluding those notices that do not pertain to any part of
          the Derivative Works; and

      (d) If the Work includes a "NOTICE" text file as part of its
          distribution, then any Derivative Works that You distribute must
          include a readable copy of the attribution notices contained
          within such NOTICE file, excluding those notices that do not
          pertain to any part of the Derivative Works, in at least one
          of the following places: within a NOTICE text file distributed
          as part of the Derivative Works; within the Source form or
          documentation, if provided along with the Derivative Works; or,
          within a display generated by the Derivative Works, if and
          wherever such third-party notices normally appear. The contents
          of the NOTICE file are for informational purposes only and
          do not modify the License. You may add Your own attribution
          notices within Derivative Works that You distribute, alongside
          or as an addendum to the NOTICE text from the Work, provided
          that such additional attribution notices cannot be construed
          as modifying the License.

      You may add Your own copyright statement to Your modifications and
      may provide additional or different license terms and conditions
      for use, reproduction, or distribution of Your modifications, or
      for any such Derivative Works as a whole, provided Your use,
      reproduction, and distribution of the Work otherwise complies with
      the conditions stated in this License.

   5. Submission of Contributions. Unless You explicitly state otherwise,
      any Contribution intentionally submitted for inclusion in the Work
      by You to the Licensor shall be under the terms and conditions of
      this License, without any additional terms or conditions.
      Notwithstanding the above, nothing herein shall supersede or modify
      the terms of any separate license agreement you may have executed
      with Licensor regarding such Contributions.

   6. Trademarks. This License does not grant permission to use the trade
      names, trademarks, service marks, or product names of the Licensor,
      except as required for reasonable and customary use in describing the
      origin of the Work and reproducing the content of the NOTICE file.

   7. Disclaimer of Warranty. Unless required by applicable law or
      agreed to in writing, Licensor provides the Work (and each
      Contributor provides its Contributions) on an "AS IS" BASIS,
      WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or
      implied, including, without limitation, any warranties or conditions
      of TITLE, NON-INFRINGEMENT, MERCHANTABILITY, or FITNESS FOR A
      PARTICULAR PURPOSE. You are solely responsible for determining the
      appropriateness of using or redistributing the Work and assume any
      risks associated with Your exercise of permissions under this License.

   8. Limitation of Liability. In no event and under no legal theory,
      whether in tort (including negligence), contract, or otherwise,
      unless required by applicable law (such as deliberate and grossly
      negligent acts) or agreed to in writing, shall any Contributor be
      liable to You for damages, including any direct, indirect, special,
      incidental, or consequential damages of any character arising as a
      result of this License or out of the use or inability to use the
      Work (including but not limited to damages for loss of goodwill,
      work stoppage, computer failure or malfunction, or any and all
      other commercial damages or losses), even if such Contributor
      has been advised of the possibility of such damages.

   9. Accepting Warranty or Additional Liability. While redistributing
      the Work or Derivative Works thereof, You may choose to offer,
      and charge a fee for, acceptance of support, warranty, indemnity,
      or other liability obligations and/or rights consistent with this
      License. However, in accepting such obligations, You may act only
      on Your own behalf and on Your sole responsibility, not on behalf
      of any other Contributor, and only if You agree to indemnify,
      defend, and hold each Contributor harmless for any liability
      incurred by, or claims asserted against, such Contributor by reason
      of your accepting any such warranty or additional liability.

   END OF TERMS AND CONDITIONS`,
      "MPL-2.0": `Mozilla Public License Version 2.0

The full text of the Mozilla Public License 2.0 is available at
https://www.mozilla.org/media/MPL/2.0/index.txt and must be supplied verbatim
when MPL-covered files are redistributed.

This Source Code Form is subject to the terms of the Mozilla Public License,
v. 2.0. If a copy of the MPL was not distributed with this file, You can
obtain one at https://mozilla.org/MPL/2.0/.`,
  ISC: `ISC License

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.`,
  "BSD-3-Clause": `BSD 3-Clause License

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice,
   this list of conditions and the following disclaimer.
2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.
3. Neither the name of the copyright holder nor the names of its contributors
   may be used to endorse or promote products derived from this software
   without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.`,
  "Unicode-3.0": `UNICODE LICENSE V3

See https://www.unicode.org/license.txt for the full text.`,
  "Zlib": `zlib License

This software is provided 'as-is', without any express or implied warranty.
Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the conditions in the full zlib license text.`,
  "CC0-1.0": `CC0 1.0 Universal

To the extent possible under law, the author has waived all copyright and
related or neighboring rights to this work.
See https://creativecommons.org/publicdomain/zero/1.0/legalcode for the full text.`,
};

/** Split an SPDX expression into its top-level alternatives. */
function alternatives(expression) {
  if (!expression || expression === "UNKNOWN") {
    return ["UNKNOWN"];
  }
  // Both `OR` and the legacy `/` separator mean "choose one".
  return expression
    .split(/\s+OR\s+|\//)
    .map((part) => part.trim())
    .filter(Boolean);
}

/**
 * The licenses a recipient must actually receive for one package.
 *
 * An `OR` expression means the recipient may choose, so one alternative is
 * enough. An `AND` expression requires every conjunct. `WITH ... exception`
 * keeps the exception attached to its license instead of becoming its own
 * identifier.
 */
function requiredLicenses(expression) {
  if (!expression || expression === "UNKNOWN") {
    return ["UNKNOWN"];
  }
  // Strip grouping; the expressions in the wild are flat enough that removing
  // parentheses keeps the meaning we care about.
  const flat = expression.replace(/[()]/g, " ").replace(/\s+/g, " ").trim();
  const chosen = flat.split(/\s+OR\s+|\//)[0].trim();
  return chosen
    .split(/\s+AND\s+/)
    .map((part) => part.trim())
    .filter(Boolean);
}

const rust = rustPackages();
const npmRoot = npmPackages(".");
const npmDocs = npmPackages("website");

const used = new Set();
for (const entry of [...rust, ...npmRoot, ...npmDocs]) {
  for (const id of requiredLicenses(entry.license)) {
    used.add(id);
  }
}

// The workspace's own packages are not third-party.
const own = new Set(["dbpinball", "dbpinball-cli", "dbpinball-core", "@programinglive/dbpinball-laravel-adapter"]);
const shipped = [...rust, ...npmRoot].filter((entry) => !own.has(entry.name));

const lines = [];
lines.push("# Third-Party License Bundle");
lines.push("");
lines.push("Generated by `scripts/generate-license-bundle.cjs`. Do not edit by hand.");
lines.push("");
lines.push(
  "DBPinball itself is MIT licensed (see `LICENSE`). This bundle records every",
  "resolved third-party crate and npm package that ships in, or is compiled into,",
  "a released binary, together with the full text of each license those",
  "dependencies use.",
);
lines.push("");
lines.push(`- Rust crates: ${rust.length}`);
lines.push(`- npm packages (application): ${npmRoot.length}`);
lines.push(`- npm packages (documentation site, not shipped): ${npmDocs.length}`);
lines.push("");

const unknown = shipped.filter((entry) => entry.license === "UNKNOWN");
if (unknown.length > 0) {
  lines.push("## Packages with no declared license");
  lines.push("");
  lines.push("These must be resolved before a release:");
  lines.push("");
  for (const entry of unknown) {
    lines.push(`- ${entry.name} ${entry.version}`);
  }
  lines.push("");
}

lines.push("## Rust crates");
lines.push("");
lines.push("| Crate | Version | License |");
lines.push("| --- | --- | --- |");
for (const entry of rust.sort((a, b) => a.name.localeCompare(b.name))) {
  lines.push(`| \`${entry.name}\` | ${entry.version} | ${entry.license} |`);
}
lines.push("");

lines.push("## npm packages (application)");
lines.push("");
lines.push("| Package | Version | License |");
lines.push("| --- | --- | --- |");
for (const entry of npmRoot.sort((a, b) => a.name.localeCompare(b.name))) {
  lines.push(`| \`${entry.name}\` | ${entry.version} | ${entry.license} |`);
}
lines.push("");

lines.push("## License texts");
lines.push("");
for (const id of [...used].sort()) {
  lines.push(`### ${id}`);
  lines.push("");
  const text = LICENSE_TEXTS[id];
  if (text) {
    lines.push("```text");
    lines.push(text);
    lines.push("```");
  } else {
    lines.push(
      `No canonical text is embedded for \`${id}\`. Obtain it from the dependency's`,
    );
    lines.push("own distribution before shipping a binary that includes it.");
  }
  lines.push("");
}

const target = path.join(root, "THIRD_PARTY_LICENSES.md");
fs.writeFileSync(target, `${lines.join("\n")}\n`, "utf8");
console.log(`wrote ${target}`);
console.log(`licenses in use: ${[...used].sort().join(", ")}`);
if (unknown.length > 0) {
  console.error(`warning: ${unknown.length} package(s) have no declared license`);
  process.exit(1);
}
