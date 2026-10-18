import http from "node:http";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const web = path.join(root, "web");
const mime = { ".html": "text/html; charset=utf-8", ".js": "text/javascript; charset=utf-8", ".css": "text/css; charset=utf-8", ".json": "application/json; charset=utf-8", ".bin": "application/octet-stream", ".svg": "image/svg+xml" };
http.createServer(async (req, res) => {
  try {
    const pathname = decodeURIComponent(new URL(req.url, "http://localhost").pathname);
    const name = pathname === "/" ? "index.html" : pathname.slice(1);
    const filename = name === "catalog.json" ? path.join(root, "catalog/catalog.json") : name === "installer.js" ? path.join(web, "dist/installer.js") : path.resolve(web, name);
    if (!filename.startsWith(root + path.sep)) throw new Error("Invalid path");
    let data;
    try {
      data = await readFile(filename);
    } catch {
      data = await readFile(path.join(root, "dist/firmware", name));
    }
    res.writeHead(200, { "Content-Type": mime[path.extname(name)] ?? "application/octet-stream", "Cache-Control": "no-store" });
    res.end(data);
  } catch {
    res.writeHead(404);
    res.end("Not found");
  }
}).listen(4197, "127.0.0.1", () => console.log("http://127.0.0.1:4197"));
