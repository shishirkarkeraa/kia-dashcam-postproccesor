import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";

const destination = resolve(process.argv[2] ?? "");
const privateKey = process.env.CLI_UPDATE_PRIVATE_KEY_BASE64;
const publicKey = process.env.KIA_CLI_UPDATE_PUBLIC_KEY_HEX;
if (!process.argv[2] || !privateKey || !publicKey) {
  throw new Error(
    "destination, CLI_UPDATE_PRIVATE_KEY_BASE64, and KIA_CLI_UPDATE_PUBLIC_KEY_HEX are required",
  );
}
if (!/^[0-9a-fA-F]{64}$/.test(publicKey)) {
  throw new Error("KIA_CLI_UPDATE_PUBLIC_KEY_HEX must contain exactly 32 bytes");
}
await mkdir(destination, { recursive: true });
await writeFile(resolve(destination, "private.key"), Buffer.from(privateKey, "base64"));
await writeFile(resolve(destination, "public.key"), Buffer.from(publicKey, "hex"));
