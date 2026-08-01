#!/usr/bin/env node

import {
  chmodSync,
  lstatSync,
  readFileSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { dirname, isAbsolute, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const MAX_PACKAGES = 128;
const MAX_LICENSE_FILES = 8;
const MAX_LICENSE_BYTES = 256 * 1024;
const MAX_OUTPUT_BYTES = 4 * 1024 * 1024;

const treePath = process.argv[2];
const output = process.argv[3];
if (!treePath || !output || !isAbsolute(treePath) || !isAbsolute(output)) {
  throw new Error("usage: generate-npm-notices.mjs ABSOLUTE_TREE ABSOLUTE_OUTPUT");
}

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const frontendRoot = resolve(projectRoot, "crates/overcrow-control-ui");
const modulesRoot = realpathSync(resolve(frontendRoot, "node_modules"));
const treeBytes = readFileSync(treePath);
if (treeBytes.length === 0 || treeBytes.length > 8 * 1024 * 1024) {
  throw new Error("npm dependency tree is empty or oversized");
}
const tree = JSON.parse(treeBytes.toString("utf8"));

const packages = new Map();
let visitedNodes = 0;

function collect(node, depth = 0) {
  visitedNodes += 1;
  if (!node || typeof node !== "object" || depth > 64 || visitedNodes > 512) {
    throw new Error("npm returned an invalid dependency tree");
  }

  if (node.path !== frontendRoot) {
    const name = node.name;
    const version = node.version;
    const license = node.license;
    if (
      typeof name !== "string" ||
      typeof version !== "string" ||
      typeof license !== "string" ||
      name.length > 256 ||
      version.length > 128 ||
      license.length > 256
    ) {
      throw new Error("npm dependency metadata is incomplete or oversized");
    }

    const packageRoot = realpathSync(node.path);
    const packageRelative = relative(modulesRoot, packageRoot);
    if (
      packageRelative === "" ||
      packageRelative.startsWith("..") ||
      isAbsolute(packageRelative) ||
      !lstatSync(packageRoot).isDirectory()
    ) {
      throw new Error(`npm dependency is outside node_modules: ${name}`);
    }

    const licenseFiles = readdirSync(packageRoot, { withFileTypes: true })
      .filter(
        (entry) =>
          entry.isFile() && /^(license|copying|notice)(?:[._-].*)?$/i.test(entry.name),
      )
      .map((entry) => entry.name)
      .sort();
    if (licenseFiles.length === 0 || licenseFiles.length > MAX_LICENSE_FILES) {
      throw new Error(`unexpected license-file count for ${name}`);
    }

    const notices = licenseFiles.map((filename) => {
      const contents = readFileSync(resolve(packageRoot, filename));
      if (contents.length === 0 || contents.length > MAX_LICENSE_BYTES || contents.includes(0)) {
        throw new Error(`invalid license file for ${name}: ${filename}`);
      }
      return {
        filename,
        contents: contents.toString("utf8").replaceAll("\r\n", "\n").trim(),
      };
    });

    const key = `${name}@${version}`;
    if (!packages.has(key)) {
      packages.set(key, { name, version, license, notices });
      if (packages.size > MAX_PACKAGES) {
        throw new Error("npm production dependency count exceeds the notice limit");
      }
    }
  }

  for (const dependency of Object.values(node.dependencies ?? {})) {
    collect(dependency, depth + 1);
  }
}

collect(tree);

const lines = ["## JavaScript packages", ""];
const orderedPackages = [...packages.values()].sort((left, right) => {
  const leftKey = `${left.name}@${left.version}`;
  const rightKey = `${right.name}@${right.version}`;
  return leftKey < rightKey ? -1 : leftKey > rightKey ? 1 : 0;
});
for (const entry of orderedPackages) {
  lines.push(`### ${entry.name} ${entry.version}`, "", `Declared license: ${entry.license}`, "");
  for (const notice of entry.notices) {
    lines.push(`#### ${notice.filename}`, "");
    for (const line of notice.contents.split("\n")) {
      lines.push(`    ${line}`);
    }
    lines.push("");
  }
}

const rendered = `${lines.join("\n")}\n`;
if (Buffer.byteLength(rendered) > MAX_OUTPUT_BYTES) {
  throw new Error("npm notices exceed the output limit");
}
writeFileSync(output, rendered, { encoding: "utf8", mode: 0o600, flag: "wx" });
chmodSync(output, 0o644);
