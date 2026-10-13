// Guards releaseEvidence package.version against package.json drift (#1654).
import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync, mkdtempSync, writeFileSync, mkdirSync, rmSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { tmpdir } from "node:os";
import { execFileSync } from "node:child_process";
import { PACKAGE_VERSION, EmbedderTestDouble } from "../dist/index.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

test("PACKAGE_VERSION matches package.json version", () => {
  assert.equal(PACKAGE_VERSION, pkg.version);
});

test("releaseEvidence().package.version matches package.json version", () => {
  const evidence = new EmbedderTestDouble().releaseEvidence();
  assert.equal(evidence.package.version, pkg.version);
  assert.equal(evidence.package.version, PACKAGE_VERSION);
});

test("verify-packed-version fails when dist PACKAGE_VERSION drifts", () => {
  const dir = mkdtempSync(join(tmpdir(), "traverse-version-neg-"));
  const tarball = join(dir, "fake.tgz");
  try {
    const packageDir = join(dir, "package");
    const distDir = join(packageDir, "dist");
    mkdirSync(distDir, { recursive: true });
    writeFileSync(
      join(distDir, "packageVersion.generated.js"),
      'export const PACKAGE_VERSION = "0.0.0-drift";\n',
    );
    writeFileSync(join(packageDir, "package.json"), JSON.stringify({ name: "x", version: pkg.version }));
    execFileSync("tar", ["czf", tarball, "-C", dir, "package"]);

    let failed = false;
    let output = "";
    try {
      execFileSync("node", [join(root, "scripts/verify-packed-version.mjs"), tarball], {
        cwd: root,
        encoding: "utf8",
      });
    } catch (error) {
      failed = true;
      output = `${error.stderr ?? ""}${error.stdout ?? ""}${error.message ?? ""}`;
    }
    assert.equal(failed, true, "verify-packed-version must reject a drifted tarball");
    assert.match(output, /0\.0\.0-drift/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
