import { readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const version = (process.argv[2] ?? "").replace(/^v/, "");

if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
  throw new Error("usage: node scripts/set-version.mjs <vMAJOR.MINOR.PATCH>");
}

const cargoPath = resolve(projectRoot, "Cargo.toml");
const cargo = await readFile(cargoPath, "utf8");
const updatedCargo = cargo.replace(
  /(\[workspace\.package\]\s*\nversion = ")[^"]+("\s*\n)/,
  (_match, prefix, suffix) => `${prefix}${version}${suffix}`,
);
if (updatedCargo === cargo) throw new Error("workspace version was not found in Cargo.toml");
await writeFile(cargoPath, updatedCargo);

const lockPath = resolve(projectRoot, "Cargo.lock");
let cargoLock = await readFile(lockPath, "utf8");
for (const packageName of ["kia-dashcam-core", "kia-dashcam-cli", "kia-dashcam-gui"]) {
  const pattern = new RegExp(`(name = "${packageName}"\\nversion = ")[^"]+("\\n)`);
  if (!pattern.test(cargoLock)) throw new Error(`${packageName} was not found in Cargo.lock`);
  cargoLock = cargoLock.replace(
    pattern,
    (_match, prefix, suffix) => `${prefix}${version}${suffix}`,
  );
}
await writeFile(lockPath, cargoLock);

for (const path of [
  resolve(projectRoot, "apps/dashcam-gui/package.json"),
  resolve(projectRoot, "apps/dashcam-gui/package-lock.json"),
]) {
  const value = JSON.parse(await readFile(path, "utf8"));
  value.version = version;
  if (value.packages?.[""]) value.packages[""].version = version;
  await writeFile(path, `${JSON.stringify(value, null, 2)}\n`);
}

const tauriPath = resolve(projectRoot, "apps/dashcam-gui/src-tauri/tauri.conf.json");
const tauri = JSON.parse(await readFile(tauriPath, "utf8"));
tauri.version = version;
await writeFile(tauriPath, `${JSON.stringify(tauri, null, 2)}\n`);

console.log(`Prepared Kia Dashcam Processor ${version}`);
