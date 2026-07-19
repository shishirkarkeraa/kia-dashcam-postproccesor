import { readFile, rm, writeFile } from "node:fs/promises";
import { basename, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const [archiveArgument, kind, keyArgument] = process.argv.slice(2);
if (!archiveArgument || !new Set(["zip", "tar"]).has(kind) || !keyArgument) {
  throw new Error("usage: node scripts/assert-tampered-update-rejected.mjs <archive> <zip|tar> <public-key>");
}
const archive = resolve(archiveArgument);
const tampered = `${archive}.tampered`;
const bytes = await readFile(archive);
bytes[Math.max(16, Math.floor(bytes.length / 2))] ^= 0x01;
await writeFile(tampered, bytes);
const verification = spawnSync(
  "zipsign",
  ["verify", kind, "--context", basename(archive), tampered, resolve(keyArgument)],
  { stdio: "ignore" },
);
await rm(tampered, { force: true });
if (verification.status === 0) throw new Error("tampered update unexpectedly passed signature verification");
console.log("Tampered CLI update rejected as expected");
