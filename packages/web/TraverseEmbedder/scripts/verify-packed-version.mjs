#!/usr/bin/env node
/**
 * After `npm pack`, fail if dist reports a PACKAGE_VERSION that does not
 * match package.json (and optionally the web-v* release tag).
 *
 * Usage:
 *   node scripts/verify-packed-version.mjs [tarball.tgz] [--expect-tag web-vX.Y.Z]
 */
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
let tarball = null;
let expectTag = null;
for (let i = 0; i < args.length; i += 1) {
  if (args[i] === "--expect-tag") {
    expectTag = args[i + 1] ?? null;
    i += 1;
    continue;
  }
  if (!args[i].startsWith("-")) {
    tarball = args[i];
  }
}

const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const expectedVersion = pkg.version;
if (typeof expectedVersion !== "string" || expectedVersion.length === 0) {
  console.error("package.json is missing a non-empty version field");
  process.exit(1);
}

if (expectTag !== null && expectTag !== `web-v${expectedVersion}`) {
  console.error(
    `expected tag web-v${expectedVersion} but got ${expectTag}`,
  );
  process.exit(1);
}

let wePacked = false;
if (tarball === null) {
  const packed = execFileSync("npm", ["pack", "--json"], {
    cwd: root,
    encoding: "utf8",
  });
  const entries = JSON.parse(packed);
  if (!Array.isArray(entries) || entries.length === 0 || !entries[0].filename) {
    console.error("npm pack --json did not return a filename");
    process.exit(1);
  }
  tarball = join(root, entries[0].filename);
  wePacked = true;
}

const extractDir = mkdtempSync(join(tmpdir(), "traverse-web-pack-"));
try {
  execFileSync("tar", ["xzf", tarball, "-C", extractDir], { stdio: "inherit" });
  const distTypes = join(extractDir, "package", "dist", "types.js");
  const distGenerated = join(
    extractDir,
    "package",
    "dist",
    "packageVersion.generated.js",
  );
  let source;
  try {
    source = readFileSync(distGenerated, "utf8");
  } catch {
    source = readFileSync(distTypes, "utf8");
  }
  const match = source.match(/PACKAGE_VERSION\s*=\s*["']([^"']+)["']/);
  if (!match) {
    console.error(`PACKAGE_VERSION not found in packed dist (tarball=${tarball})`);
    process.exit(1);
  }
  const packedVersion = match[1];
  if (packedVersion !== expectedVersion) {
    console.error(
      `packed PACKAGE_VERSION ${packedVersion} !== package.json ${expectedVersion}`,
    );
    process.exit(1);
  }
  console.log(
    `packed PACKAGE_VERSION ${packedVersion} matches package.json / tag expectation`,
  );
} finally {
  rmSync(extractDir, { recursive: true, force: true });
  if (wePacked) {
    rmSync(tarball, { force: true });
  }
}
