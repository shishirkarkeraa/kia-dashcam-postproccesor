import { mkdir, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repository = process.env.KIA_DASHCAM_UPDATE_REPO;
const publicKey = process.env.TAURI_SIGNING_PUBLIC_KEY;
if (!repository || !/^[^/]+\/[^/]+$/.test(repository) || !publicKey) {
  throw new Error("KIA_DASHCAM_UPDATE_REPO=OWNER/REPO and TAURI_SIGNING_PUBLIC_KEY are required");
}

const output = resolve(
  projectRoot,
  "apps/dashcam-gui/src-tauri/tauri.release.conf.json",
);
const config = {
  bundle: {
    createUpdaterArtifacts: true,
    ...(process.env.APPLE_SIGNING_IDENTITY
      ? {
          macOS: {
            signingIdentity: process.env.APPLE_SIGNING_IDENTITY,
            hardenedRuntime: true,
          },
        }
      : {}),
  },
  plugins: {
    updater: {
      pubkey: publicKey,
      endpoints: [`https://github.com/${repository}/releases/latest/download/latest.json`],
    },
  },
};
await mkdir(dirname(output), { recursive: true });
await writeFile(output, `${JSON.stringify(config, null, 2)}\n`);
console.log(`Prepared signed updater configuration for ${repository}`);
