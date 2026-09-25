#!/usr/bin/env node
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL(".", import.meta.url));
const repo = fileURLToPath(new URL("../../../../../", import.meta.url));
const dist = join(repo, "packages/web/TraverseEmbedder/dist");
const safe = (path) => normalize(path).replace(/^([.][.][/\\])+/, "").replace(/^\/+/, "");
const contentType = (path) => ({ ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8", ".wasm": "application/wasm" }[extname(path)] ?? "application/octet-stream");

createServer(async (request, response) => {
  const path = request.url?.split("?")[0] ?? "/";
  const source = path.startsWith("/pkg/") ? join(dist, safe(path.slice(5)))
    : path.startsWith("/repo/") ? join(repo, safe(path.slice(6)))
      : join(root, path === "/" ? "index.html" : safe(path));
  try {
    response.setHeader("Content-Type", contentType(source));
    response.end(await readFile(source));
  } catch {
    response.statusCode = 404;
    response.end("Not found");
  }
}).listen(4177, "127.0.0.1", () => console.log("Audio analysis: http://127.0.0.1:4177"));
