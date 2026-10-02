#!/usr/bin/env node
// Decision 103 (#1567): generate the production Spec 138 model-signing key.
//
// MAINTAINER-ONLY. Writes the 32-byte Ed25519 secret as 64 hex characters to
// --secret-out (which must be OUTSIDE this repository, mode 0600) and the
// public key to keys/model-signing/<key_id>.pub. Then:
//   1. upload the secret file's contents as the `MODEL_SIGNING_KEY_HEX` secret
//      of the protected `model-signing` GitHub Environment;
//   2. securely delete the local secret file;
//   3. commit keys/model-signing/<key_id>.pub and add it to
//      TRAVERSE_MODEL_SIGNING_KEYS (see docs/model-signing-key-runbook.md).
//
//   node scripts/model-signing/generate-key.mjs --secret-out ~/model-signing.secret.hex
import { createHash, generateKeyPairSync } from "node:crypto";
import { existsSync, realpathSync, writeFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = realpathSync(join(dirname(fileURLToPath(import.meta.url)), "..", ".."));
const flag = process.argv.indexOf("--secret-out");
const secretOut = flag > 0 ? process.argv[flag + 1] : undefined;
if (!secretOut) {
  console.error("usage: generate-key.mjs --secret-out <path outside the repository>");
  process.exit(2);
}
const secretPath = resolve(secretOut);
const secretDir = realpathSync(dirname(secretPath));
if (secretDir === root || secretDir.startsWith(root + sep)) {
  console.error("refusing to write the private key inside the repository");
  process.exit(2);
}
if (existsSync(secretPath)) {
  console.error(`refusing to overwrite ${secretPath}`);
  process.exit(2);
}

const { privateKey, publicKey } = generateKeyPairSync("ed25519");
const secretHex = Buffer.from(privateKey.export({ format: "jwk" }).d, "base64url").toString("hex");
const publicRaw = Buffer.from(publicKey.export({ format: "jwk" }).x, "base64url");
const keyId = `ed25519:${createHash("sha256").update(publicRaw).digest("hex")}`;

writeFileSync(secretPath, `${secretHex}\n`, { mode: 0o600, flag: "wx" });
const publicPath = join(root, "keys", "model-signing", `${keyId.replace(":", "-")}.pub`);
writeFileSync(publicPath, `${publicRaw.toString("hex")}\n`, { flag: "wx" });

console.log(JSON.stringify({ key_id: keyId, public_key_hex: publicRaw.toString("hex"), public_key_path: publicPath, secret_path: secretPath }, null, 2));
