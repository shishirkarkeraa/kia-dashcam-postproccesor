import { cp, mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const options = parseOptions(process.argv.slice(2));
for (const required of ["target", "binary", "media", "output", "kind"]) {
  if (!options[required]) throw new Error(`missing --${required}`);
}
if (!new Set(["zip", "tar"]).has(options.kind)) {
  throw new Error("--kind must be zip or tar");
}

const output = resolve(options.output);
const staging = await mkdtemp(join(tmpdir(), "kia-dashcam-cli-package-"));
const executable = process.platform === "win32" ? "kia-dashcam-cli.exe" : "kia-dashcam-cli";
await mkdir(dirname(output), { recursive: true });
await cp(resolve(options.binary), join(staging, executable));
await cp(resolve(options.media), join(staging, "media-tools"), { recursive: true });
await cp(resolve("THIRD_PARTY_NOTICES.md"), join(staging, "THIRD_PARTY_NOTICES.md"));
await writeFile(
  join(staging, "README.txt"),
  [
    "Kia Dashcam Processor CLI",
    `Target: ${options.target}`,
    "",
    "Keep the media-tools folder next to the CLI executable.",
    "Run: kia-dashcam-cli process <folder> --cleanup keep",
    "",
  ].join("\n"),
);

const archiveArgs = options.kind === "zip"
  ? ["-a", "-cf", output, "-C", staging, "."]
  : ["-czf", output, "-C", staging, "."];
const archive = spawnSync("tar", archiveArgs, { stdio: "inherit" });
if (archive.status !== 0) throw new Error(`tar failed with exit code ${archive.status}`);
console.log(`Created ${basename(output)}`);

function parseOptions(args) {
  const values = {};
  for (let index = 0; index < args.length; index += 2) {
    const key = args[index]?.replace(/^--/, "");
    const value = args[index + 1];
    if (!key || value === undefined) throw new Error(`invalid argument: ${args[index] ?? ""}`);
    values[key] = value;
  }
  return values;
}
