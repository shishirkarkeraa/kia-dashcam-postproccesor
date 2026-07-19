import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDir, "..");
const guiRoot = join(projectRoot, "apps", "dashcam-gui");
const lock = JSON.parse(
  await readFile(join(projectRoot, "media-tools.lock.json"), "utf8"),
);
const targetPlatform = process.env.KIA_TARGET_PLATFORM || process.platform;
const targetArch = process.env.KIA_TARGET_ARCH || process.arch;
const targetKey = `${targetPlatform}-${targetArch}`;
const targetConfig = lock.ffmpeg.targets[targetKey];
const finalizeOnly = process.argv.includes("--finalize-only");
const destinationArgument = process.argv.indexOf("--destination");
const destination =
  destinationArgument >= 0
    ? resolve(process.argv[destinationArgument + 1])
    : join(guiRoot, "src-tauri", "binaries", "media-tools");
const executableSuffix = targetPlatform === "win32" ? ".exe" : "";

if (!targetConfig) {
  throw new Error(`Unsupported media-tool target: ${targetKey}`);
}
if (targetPlatform !== process.platform || targetArch !== process.arch) {
  throw new Error(
    `Media tools must be staged on their native runner. Requested ${targetKey}, running ${process.platform}-${process.arch}.`,
  );
}

await mkdir(destination, { recursive: true });
if (!finalizeOnly) {
  for (const name of [
    `ffmpeg${executableSuffix}`,
    `ffprobe${executableSuffix}`,
    `HandBrakeCLI${executableSuffix}`,
    "media-tools.json",
    "THIRD_PARTY_NOTICES.md",
  ]) {
    await rm(join(destination, name), { force: true });
  }
}

const temporary = await mkdtemp(join(tmpdir(), "kia-media-tools-"));
try {
  const ffmpegDestination = join(destination, `ffmpeg${executableSuffix}`);
  const ffprobeDestination = join(destination, `ffprobe${executableSuffix}`);
  const handbrakeDestination = join(
    destination,
    `HandBrakeCLI${executableSuffix}`,
  );
  if (!finalizeOnly) {
    await acquireFfmpegTools(
      targetConfig,
      temporary,
      ffmpegDestination,
      ffprobeDestination,
    );

    const handbrakeSource = process.env.KIA_HANDBRAKE_PATH
      ? resolve(process.env.KIA_HANDBRAKE_PATH)
      : await acquireHandBrake(temporary);
    await copyExecutable(handbrakeSource, handbrakeDestination);
  }

  if (targetPlatform === "darwin") {
    for (const binary of [
      ffmpegDestination,
      ffprobeDestination,
      handbrakeDestination,
    ]) {
      await thinMacosExecutable(binary);
    }
  }

  auditArchitecture(ffmpegDestination, "FFmpeg");
  auditArchitecture(ffprobeDestination, "FFprobe");
  auditArchitecture(handbrakeDestination, "HandBrakeCLI");
  const ffmpegLicense = auditFfmpeg(ffmpegDestination, "FFmpeg");
  const ffprobeLicense = auditFfmpeg(ffprobeDestination, "FFprobe");
  const handbrakeVersion = auditHandBrake(handbrakeDestination);
  const codeSigning = signMacosExecutables([
    ffmpegDestination,
    ffprobeDestination,
    handbrakeDestination,
  ]);

  const staged = {
    target: targetKey,
    generatedAt: new Date().toISOString(),
    runtimePolicy: "bundled-only-no-path-search-no-runtime-download",
    ...(codeSigning ? { codeSigning } : {}),
    tools: {
      ffmpeg: {
        version: targetConfig.version,
        provider: targetConfig.provider,
        sha256: await sha256(ffmpegDestination),
        licenseSummary: ffmpegLicense,
      },
      ffprobe: {
        version: targetConfig.version,
        provider: targetConfig.provider,
        sha256: await sha256(ffprobeDestination),
        licenseSummary: ffprobeLicense,
      },
      handbrake: {
        version: lock.handbrake.version,
        detectedVersion: handbrakeVersion,
        sha256: await sha256(handbrakeDestination),
      },
    },
  };
  await writeFile(
    join(destination, "media-tools.json"),
    `${JSON.stringify(staged, null, 2)}\n`,
  );
  await copyFile(
    join(projectRoot, "THIRD_PARTY_NOTICES.md"),
    join(destination, "THIRD_PARTY_NOTICES.md"),
  );
  console.log(
    `${finalizeOnly ? "Finalized" : "Staged"} checksum-verified, redistribution-audited media tools for ${targetKey} in ${destination}`,
  );
} finally {
  await rm(temporary, { recursive: true, force: true });
}

async function acquireFfmpegTools(config, temporaryRoot, ffmpegOutput, ffprobeOutput) {
  if (config.archives) {
    for (const [name, archiveConfig] of Object.entries(config.archives)) {
      const archive = join(temporaryRoot, `${name}.zip`);
      const extracted = join(temporaryRoot, `${name}-extracted`);
      await downloadAndVerify(
        archiveConfig.url,
        archive,
        archiveConfig.sha256,
      );
      await extractZip(archive, extracted);
      const source = await findNamedFile(
        extracted,
        `${name}${executableSuffix}`,
      );
      if (!source) {
        throw new Error(`${name}${executableSuffix} was not found in ${basename(archive)}`);
      }
      await copyExecutable(
        source,
        name === "ffmpeg" ? ffmpegOutput : ffprobeOutput,
      );
    }
    return;
  }

  const archive = join(temporaryRoot, "ffmpeg-suite.zip");
  const extracted = join(temporaryRoot, "ffmpeg-suite-extracted");
  await downloadAndVerify(config.archive.url, archive, config.archive.sha256);
  await extractZip(archive, extracted);
  const ffmpegSource = await findNamedFile(
    extracted,
    `ffmpeg${executableSuffix}`,
  );
  const ffprobeSource = await findNamedFile(
    extracted,
    `ffprobe${executableSuffix}`,
  );
  if (!ffmpegSource || !ffprobeSource) {
    throw new Error("The pinned FFmpeg archive does not contain both FFmpeg and FFprobe");
  }
  await copyExecutable(ffmpegSource, ffmpegOutput);
  await copyExecutable(ffprobeSource, ffprobeOutput);
}

async function acquireHandBrake(temporaryRoot) {
  if (targetPlatform === "linux") {
    throw new Error(
      "Linux builds must compile the pinned HandBrake commit and set KIA_HANDBRAKE_PATH before staging.",
    );
  }
  if (targetPlatform === "darwin") {
    const archive = join(temporaryRoot, "HandBrakeCLI.dmg");
    await downloadAndVerify(
      lock.handbrake.darwin.url,
      archive,
      lock.handbrake.darwin.sha256,
    );
    const mountpoint = join(temporaryRoot, "handbrake-mounted");
    await mkdir(mountpoint);
    run("hdiutil", [
      "attach",
      archive,
      "-nobrowse",
      "-readonly",
      "-mountpoint",
      mountpoint,
    ]);
    try {
      const binary = await findNamedFile(mountpoint, "HandBrakeCLI");
      if (!binary) throw new Error("HandBrakeCLI was not found in the official DMG");
      const copied = join(temporaryRoot, "HandBrakeCLI");
      await copyExecutable(binary, copied);
      return copied;
    } finally {
      run("hdiutil", ["detach", mountpoint]);
    }
  }
  if (targetPlatform === "win32" && targetArch === "x64") {
    const archive = join(temporaryRoot, "HandBrakeCLI.zip");
    const extracted = join(temporaryRoot, "handbrake-extracted");
    await downloadAndVerify(
      lock.handbrake["win32-x64"].url,
      archive,
      lock.handbrake["win32-x64"].sha256,
    );
    await extractZip(archive, extracted);
    const binary = await findNamedFile(extracted, "HandBrakeCLI.exe");
    if (!binary) {
      throw new Error("HandBrakeCLI.exe was not found in the official ZIP");
    }
    return binary;
  }
  throw new Error(`HandBrake is not configured for ${targetKey}`);
}

async function extractZip(archive, output) {
  await mkdir(output, { recursive: true });
  if (targetPlatform === "win32") {
    run(
      "powershell",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "Expand-Archive -LiteralPath $env:KIA_MEDIA_ARCHIVE -DestinationPath $env:KIA_MEDIA_OUTPUT -Force",
      ],
      {
        env: {
          ...process.env,
          KIA_MEDIA_ARCHIVE: archive,
          KIA_MEDIA_OUTPUT: output,
        },
      },
    );
  } else {
    run("unzip", ["-q", "-o", archive, "-d", output]);
  }
}

async function downloadAndVerify(url, output, expectedHash) {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`Download failed (${response.status}) for ${url}`);
  }
  await writeFile(output, Buffer.from(await response.arrayBuffer()));
  const actualHash = await sha256(output);
  if (actualHash !== expectedHash) {
    throw new Error(
      `Archive checksum mismatch for ${url}: expected ${expectedHash}, got ${actualHash}`,
    );
  }
}

async function copyExecutable(source, output) {
  await copyFile(source, output);
  if (targetPlatform !== "win32") await chmod(output, 0o755);
}

async function thinMacosExecutable(binary) {
  const expected = expectedMacosArchitecture();
  const architectures = macosArchitectures(binary);
  if (!architectures.includes(expected)) {
    throw new Error(
      `${basename(binary)} architecture mismatch: expected ${expected}, found ${architectures.join(", ")}`,
    );
  }
  if (architectures.length === 1) return;

  const thinned = `${binary}.thin-${process.pid}`;
  await rm(thinned, { force: true });
  try {
    run("lipo", [binary, "-thin", expected, "-output", thinned]);
    await chmod(thinned, 0o755);
    await rename(thinned, binary);
  } finally {
    await rm(thinned, { force: true });
  }
}

function signMacosExecutables(binaries) {
  if (targetPlatform !== "darwin") return undefined;

  const identity = process.env.APPLE_SIGNING_IDENTITY?.trim() || "-";
  const mode = identity === "-"
    ? "ad-hoc"
    : identity.startsWith("Developer ID Application:")
      ? "developer-id"
      : "apple-identity";
  if (
    process.env.KIA_REQUIRE_DEVELOPER_ID === "true" &&
    mode !== "developer-id"
  ) {
    throw new Error(
      `Release staging requires a Developer ID Application identity, received: ${identity}`,
    );
  }
  for (const binary of binaries) {
    const args = ["--force", "--options", "runtime"];
    if (identity !== "-") args.push("--timestamp");
    args.push("--sign", identity, binary);
    run("codesign", args);
    run("codesign", ["--verify", "--strict", "--verbose=2", binary]);
  }

  return {
    mode,
    hardenedRuntime: true,
    verified: true,
  };
}

function auditFfmpeg(binary, label) {
  const result = spawnSync(binary, ["-L"], {
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      `${label} redistribution audit could not run: ${result.error?.message || `exit ${result.status}`}`,
    );
  }
  const output = `${result.stdout || ""}\n${result.stderr || ""}`;
  if (/not legally redistributable|enable-nonfree/i.test(output)) {
    throw new Error(
      `${label} failed the redistribution gate: this build contains nonfree components`,
    );
  }
  if (!output.includes("--enable-gpl") || !output.includes("--enable-libx264")) {
    throw new Error(
      `${label} failed the feature gate: GPL and libx264 support are required`,
    );
  }
  return "GNU General Public License configuration verified; nonfree disabled";
}

function auditArchitecture(binary, label) {
  if (targetPlatform !== "darwin") return;
  const expected = expectedMacosArchitecture();
  const architectures = macosArchitectures(binary);
  if (architectures.length !== 1 || architectures[0] !== expected) {
    throw new Error(
      `${label} must contain only ${expected}, found ${architectures.join(", ")}`,
    );
  }
}

function expectedMacosArchitecture() {
  if (targetArch === "arm64") return "arm64";
  if (targetArch === "x64") return "x86_64";
  throw new Error(`Unsupported macOS architecture: ${targetArch}`);
}

function macosArchitectures(binary) {
  const result = spawnSync("lipo", ["-archs", binary], { encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw new Error(
      `${basename(binary)} architecture audit could not run: ${result.error?.message || `exit ${result.status}`}`,
    );
  }
  return result.stdout.trim().split(/\s+/).filter(Boolean);
}

function auditHandBrake(binary) {
  const result = spawnSync(binary, ["--version"], {
    encoding: "utf8",
    maxBuffer: 10 * 1024 * 1024,
  });
  if (result.error || result.status !== 0) {
    throw new Error(
      `HandBrakeCLI audit could not run: ${result.error?.message || `exit ${result.status}`}`,
    );
  }
  const output = `${result.stdout || ""}\n${result.stderr || ""}`;
  if (!output.includes(`HandBrake ${lock.handbrake.version}`)) {
    throw new Error(
      `HandBrakeCLI version mismatch: expected ${lock.handbrake.version}`,
    );
  }
  return `HandBrake ${lock.handbrake.version}`;
}

async function sha256(path) {
  return createHash("sha256").update(await readFile(path)).digest("hex");
}

async function findNamedFile(root, name) {
  for (const entry of await readdir(root, { withFileTypes: true })) {
    const path = join(root, entry.name);
    if (entry.isFile() && entry.name === name) return path;
    if (entry.isDirectory()) {
      const nested = await findNamedFile(path, name);
      if (nested) return nested;
    }
  }
  return undefined;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", ...options });
  if (result.error || result.status !== 0) {
    throw new Error(
      `${command} failed: ${result.error?.message || `exit code ${result.status}`}`,
    );
  }
}
