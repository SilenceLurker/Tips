import { readFile, access, appendFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const packageJson = JSON.parse(await readFile(path.join(root, "package.json"), "utf8"));
const tauriConfig = JSON.parse(
  await readFile(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
);
const manifest = await readFile(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
const packageSection = manifest.match(/^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))/m)?.[1];
const cargoName = packageSection?.match(/^name\s*=\s*"([^"]+)"/m)?.[1];
const cargoVersion = packageSection?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const lock = await readFile(path.join(root, "src-tauri", "Cargo.lock"), "utf8");
const lockedPackage = lock.split(/^\[\[package\]\]/m).find(
  (section) => section.match(/^name\s*=\s*"([^"]+)"/m)?.[1] === cargoName,
);
const lockedVersion = lockedPackage?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
const version = packageJson.version;
const tag = process.argv[2] || process.env.RELEASE_TAG || `v${version}`;

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?$/.test(version)) {
  throw new Error(`Invalid release version: ${version}`);
}
if (version !== tauriConfig.version || version !== cargoVersion || version !== lockedVersion) {
  throw new Error(
    `Versions must match: package.json=${version}, tauri.conf.json=${tauriConfig.version}, Cargo.toml=${cargoVersion}, Cargo.lock=${lockedVersion}`,
  );
}
if (tag !== `v${version}`) {
  throw new Error(`Tag ${tag} does not match the application version v${version}`);
}
const notesPath = path.join(root, "docs", "releases", `${tag}.md`);
await access(notesPath);
const notes = await readFile(notesPath, "utf8");
if (!notes.trim()) throw new Error(`Release notes are empty: ${notesPath}`);

// Keep early 0.x versions and explicitly suffixed versions marked as prereleases.
const prerelease = version.startsWith("0.") || version.includes("-");
if (process.env.GITHUB_OUTPUT) {
  await appendFile(
    process.env.GITHUB_OUTPUT,
    `version=${version}\ntag=${tag}\nprerelease=${prerelease}\n`,
  );
}
console.log(`Release configuration OK: ${tag}${prerelease ? " (prerelease)" : ""}`);
