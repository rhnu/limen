import { readdir, rm } from "node:fs/promises";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";

// Only top-level builtin sources are entrypoints; private imports are bundled.
// The Rust registry alone assigns their public limen: names.
const root = fileURLToPath(new URL("../../", import.meta.url));
const sources = await readdir(new URL("src/builtins/", import.meta.url), { withFileTypes: true });
const entryPoints = sources
  .filter(entry => entry.isFile() && entry.name.endsWith(".js"))
  .map(entry => `modules/script/src/builtins/${entry.name}`)
  .sort();

// Remove only this fixed generated directory, never a caller-selected path.
const outdir = resolve(root, "modules/script/dist");
if (relative(root, outdir) !== join("modules", "script", "dist")) {
  throw new Error("script output must stay inside modules/script/dist");
}
await rm(outdir, { recursive: true, force: true });

await build({
  absWorkingDir: root,
  entryPoints,
  outdir,
  bundle: true,
  format: "esm",
  platform: "neutral",
  target: "es2023",
  legalComments: "inline",
  logLevel: "info",
});
