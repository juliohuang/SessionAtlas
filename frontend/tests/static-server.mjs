import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
export const port = 4173;
const types = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
]);

function handleRequest(request, response) {
  void serveRequest(request, response);
}

async function serveRequest(request, response) {
  try {
    const requestUrl = new URL(request.url || "/", "http://127.0.0.1");
    const relativePath = decodeURIComponent(requestUrl.pathname === "/" ? "/index.html" : requestUrl.pathname)
      .replace(/^[/\\]+/, "");
    const path = resolve(root, relativePath);
    if (path !== root && !path.startsWith(`${root}${sep}`)) {
      response.writeHead(403).end("forbidden");
      return;
    }
    const info = await stat(path);
    if (!info.isFile()) throw new Error("not a file");
    response.writeHead(200, {
      "Cache-Control": "no-store",
      "Content-Type": types.get(extname(path)) || "application/octet-stream",
    });
    createReadStream(path).pipe(response);
  } catch {
    response.writeHead(404).end("not found");
  }
}

export function startStaticServer() {
  const server = createServer(handleRequest);
  return new Promise((resolvePromise, rejectPromise) => {
    const reject = error => rejectPromise(error);
    server.once("error", reject);
    server.listen(port, "127.0.0.1", () => {
      server.off("error", reject);
      resolvePromise(server);
    });
  });
}

export function closeStaticServer(server) {
  return new Promise(resolvePromise => {
    let finished = false;
    const finish = () => {
      if (finished) return;
      finished = true;
      clearTimeout(fallback);
      resolvePromise();
    };
    const fallback = setTimeout(() => {
      server.closeAllConnections?.();
      finish();
    }, 1_000);
    fallback.unref();
    server.close(finish);
    server.closeIdleConnections?.();
    server.closeAllConnections?.();
  });
}

// Keep the file useful as a standalone development server. Playwright imports
// the functions above from global setup so the server lives in its own runner
// process and does not require process-tree termination on Windows.
const isDirectRun = process.argv[1]
  && resolve(process.argv[1]) === resolve(fileURLToPath(import.meta.url));
if (isDirectRun) {
  const server = await startStaticServer();
  let closing = false;
  const close = async () => {
    if (closing) return;
    closing = true;
    await closeStaticServer(server);
    process.exit(0);
  };
  process.once("SIGINT", close);
  process.once("SIGTERM", close);
}
