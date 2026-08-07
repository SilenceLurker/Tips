import { readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourceRoot = path.join(root, "src");

const forbiddenSourcePatterns = [
  ["Vue raw HTML rendering", /\bv-html\s*=/i],
  ["innerHTML assignment", /\.innerHTML\s*=/i],
  ["outerHTML assignment", /\.outerHTML\s*=/i],
  ["insertAdjacentHTML", /\binsertAdjacentHTML\s*\(/i],
  ["document.write", /\bdocument\.write\s*\(/i],
  ["eval", /\beval\s*\(/i],
  ["Function constructor", /\bnew\s+Function\s*\(/i],
  ["javascript URL", /javascript\s*:/i],
  ["HTML data URL", /data\s*:\s*text\/html/i],
  ["iframe srcdoc", /\bsrcdoc\s*=/i],
];

async function sourceFiles(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) files.push(...(await sourceFiles(fullPath)));
    else if (/\.(?:ts|vue|js|mjs|html)$/i.test(entry.name)) files.push(fullPath);
  }
  return files;
}

const failures = [];
for (const file of await sourceFiles(sourceRoot)) {
  const content = await readFile(file, "utf8");
  const relative = path.relative(root, file);
  for (const [label, pattern] of forbiddenSourcePatterns) {
    if (pattern.test(content)) failures.push(`${relative}: forbidden ${label}`);
  }
}

const tauriConfig = JSON.parse(
  await readFile(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const security = tauriConfig.app?.security;
if (!security?.csp) failures.push("src-tauri/tauri.conf.json: production CSP is missing");
if (security?.freezePrototype !== true) {
  failures.push("src-tauri/tauri.conf.json: freezePrototype must be true");
}
if (tauriConfig.app?.withGlobalTauri !== false) {
  failures.push("src-tauri/tauri.conf.json: withGlobalTauri must stay false");
}

const capability = JSON.parse(
  await readFile(path.join(root, "src-tauri", "capabilities", "default.json"), "utf8"),
);
const forbiddenPermissions = new Set([
  "core:default",
  "core:event:default",
  "core:window:default",
  "core:webview:default",
  "opener:default",
]);
for (const permission of capability.permissions ?? []) {
  if (forbiddenPermissions.has(permission)) {
    failures.push(`src-tauri/capabilities/default.json: overbroad permission ${permission}`);
  }
}

const packageJson = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
if (packageJson.dependencies?.["@tauri-apps/plugin-opener"]) {
  failures.push("package.json: unused opener plugin must not be bundled");
}

if (failures.length) {
  console.error("Security baseline check failed:\n" + failures.map((item) => `- ${item}`).join("\n"));
  process.exitCode = 1;
} else {
  console.log("Security baseline check passed");
}
