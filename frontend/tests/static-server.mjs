import { createReadStream } from "node:fs";
import { stat } from "node:fs/promises";
import { createServer } from "node:http";
import { extname, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(fileURLToPath(new URL("..", import.meta.url)));
const port = 4173;
const types = new Map([
  [".css", "text/css; charset=utf-8"],
  [".html", "text/html; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
]);

const server = createServer(async (request, response) => {
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
});

server.listen(port, "127.0.0.1");

const close = () => server.close(() => process.exit(0));
process.once("SIGINT", close);
process.once("SIGTERM", close);
