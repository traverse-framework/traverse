#!/usr/bin/env node
// Static file server for the React integration example. Serves the built
// `traverse-embedder-web` package, the checked-in `traverse-starter`
// application bundle straight from the repository, the existing React
// vendor bundles from apps/react-demo, and this example's own page —
// nothing here is a `traverse-cli serve` sidecar; it exists only to serve
// static files to the browser (spec 068 NFR: no production sidecar
// dependency).
import { createServer } from "node:http";
import { readFile } from "node:fs/promises";
import { extname, join, normalize } from "node:path";
import { fileURLToPath } from "node:url";

const exampleRoot = fileURLToPath(new URL(".", import.meta.url));
const repoRoot = fileURLToPath(new URL("../../../../../", import.meta.url));
const pkgDist = join(repoRoot, "packages/web/TraverseEmbedder/dist");
const vendorRoot = join(repoRoot, "packages/web/TraverseEmbedder/examples/react-integration/vendor");
const defaultPort = 4175;

const routes = [
  { prefix: "/repo/", root: repoRoot },
  { prefix: "/pkg/", root: pkgDist },
  { prefix: "/vendor/", root: vendorRoot },
];

const port = parsePort(process.argv.slice(2));

const server = createServer(async (request, response) => {
  try {
    await serveStaticAsset(request, response);
  } catch (error) {
    response.statusCode = 500;
    response.setHeader("Content-Type", "text/plain; charset=utf-8");
    response.end(String(error));
  }
});

server.listen(port, "127.0.0.1", () => {
  console.log(`Traverse Web embedder React integration example serving on http://127.0.0.1:${port}`);
});

function parsePort(args) {
  const index = args.indexOf("--port");
  if (index === -1 || args[index + 1] === undefined) {
    return defaultPort;
  }
  const value = Number(args[index + 1]);
  if (!Number.isInteger(value) || value <= 0) {
    throw new Error(`invalid port: ${args[index + 1]}`);
  }
  return value;
}

function safeRelativePath(requestPath) {
  return normalize(requestPath).replace(/^([.][.][/\\])+/, "").replace(/^\/+/, "");
}

async function serveStaticAsset(request, response) {
  const requestPath = request.url?.split("?")[0] ?? "/";

  for (const route of routes) {
    if (requestPath.startsWith(route.prefix)) {
      const relative = safeRelativePath(requestPath.slice(route.prefix.length));
      await respondWithFile(response, join(route.root, relative));
      return;
    }
  }

  const relative = requestPath === "/" ? "index.html" : safeRelativePath(requestPath);
  await respondWithFile(response, join(exampleRoot, relative));
}

async function respondWithFile(response, filePath) {
  try {
    const contents = await readFile(filePath);
    response.statusCode = 200;
    response.setHeader("Content-Type", contentTypeFor(filePath));
    response.end(contents);
  } catch {
    response.statusCode = 404;
    response.setHeader("Content-Type", "text/plain; charset=utf-8");
    response.end(`Not found: ${filePath}`);
  }
}

function contentTypeFor(filePath) {
  switch (extname(filePath)) {
    case ".html":
      return "text/html; charset=utf-8";
    case ".js":
      return "text/javascript; charset=utf-8";
    case ".css":
      return "text/css; charset=utf-8";
    case ".json":
      return "application/json; charset=utf-8";
    case ".wasm":
      return "application/wasm";
    default:
      return "application/octet-stream";
  }
}
