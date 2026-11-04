// Serves this folder on 127.0.0.1 and saves the browser's results to proof/browser.json.
import { createServer } from "node:http";
import { readFile, writeFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
const TYPES = { ".html": "text/html", ".mjs": "text/javascript", ".js": "text/javascript", ".json": "application/json", ".wasm": "application/wasm" };
createServer(async (request, response) => {
  const path = normalize(new URL(request.url, "http://x").pathname).replace(/^\/+/, "") || "src/browser.html";
  if (request.method === "POST" && (path === "proof" || path === "failed")) {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    await writeFile(path === "proof" ? "proof/browser.json" : "proof/browser-failed.txt", Buffer.concat(chunks));
    response.writeHead(204).end();
    return;
  }
  try {
    const body = await readFile(join(process.cwd(), path));
    response.writeHead(200, { "content-type": TYPES[extname(path)] ?? "application/octet-stream" }).end(body);
  } catch {
    response.writeHead(404).end();
  }
}).listen(8791, "127.0.0.1", () => console.log("listening on 8791"));
