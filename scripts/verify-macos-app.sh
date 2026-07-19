#!/usr/bin/env bash
set -euo pipefail

app_path="${1:?usage: verify-macos-app.sh APP_PATH TARGET_TRIPLE}"
target_triple="${2:?usage: verify-macos-app.sh APP_PATH TARGET_TRIPLE}"

case "$target_triple" in
  aarch64-apple-darwin)
    expected_arch="arm64"
    manifest_target="darwin-arm64"
    ;;
  x86_64-apple-darwin)
    expected_arch="x86_64"
    manifest_target="darwin-x64"
    ;;
  *)
    echo "unsupported macOS target: $target_triple" >&2
    exit 1
    ;;
esac

if [[ ! -d "$app_path" ]]; then
  echo "application bundle not found: $app_path" >&2
  exit 1
fi

contents="$app_path/Contents"
main_executable="$contents/MacOS/kia-dashcam-gui"
media_dir="$contents/Resources/media-tools"
manifest="$media_dir/media-tools.json"

if [[ ! -d "$contents/_CodeSignature" ]]; then
  echo "application has no sealed bundle signature: $contents/_CodeSignature" >&2
  exit 1
fi
if [[ ! -x "$main_executable" ]]; then
  echo "main application executable is missing: $main_executable" >&2
  exit 1
fi
if [[ ! -f "$manifest" ]]; then
  echo "media-tool manifest is missing: $manifest" >&2
  exit 1
fi

codesign --verify --deep --strict --verbose=4 "$app_path"
xcrun stapler validate "$app_path"
spctl --assess --type execute --verbose=4 "$app_path"

app_details="$(codesign --display --verbose=4 "$app_path" 2>&1)"
app_team="$(awk -F= '$1 == "TeamIdentifier" { print $2 }' <<<"$app_details")"
if [[ -z "$app_team" || "$app_team" == "not set" ]]; then
  echo "application is not signed with a Developer ID team" >&2
  exit 1
fi

main_archs="$(lipo -archs "$main_executable")"
if [[ "$main_archs" != "$expected_arch" ]]; then
  echo "main executable architecture mismatch: expected $expected_arch, found $main_archs" >&2
  exit 1
fi

node -e '
  const fs = require("node:fs");
  const manifest = JSON.parse(fs.readFileSync(process.argv[1], "utf8"));
  if (manifest.target !== process.argv[2]) {
    throw new Error(`media target mismatch: expected ${process.argv[2]}, found ${manifest.target}`);
  }
  if (manifest.codeSigning?.mode !== "developer-id" || manifest.codeSigning?.verified !== true) {
    throw new Error("media tools were not finalized with verified Developer ID signatures");
  }
' "$manifest" "$manifest_target"

for tool in ffmpeg ffprobe HandBrakeCLI; do
  tool_path="$media_dir/$tool"
  test -x "$tool_path"
  codesign --verify --strict --verbose=4 "$tool_path"

  tool_archs="$(lipo -archs "$tool_path")"
  if [[ "$tool_archs" != "$expected_arch" ]]; then
    echo "$tool architecture mismatch: expected $expected_arch, found $tool_archs" >&2
    exit 1
  fi

  tool_details="$(codesign --display --verbose=4 "$tool_path" 2>&1)"
  tool_team="$(awk -F= '$1 == "TeamIdentifier" { print $2 }' <<<"$tool_details")"
  if [[ "$tool_team" != "$app_team" ]]; then
    echo "$tool signing team mismatch: expected $app_team, found ${tool_team:-none}" >&2
    exit 1
  fi
done

node -e '
  const crypto = require("node:crypto");
  const fs = require("node:fs");
  const path = require("node:path");
  const directory = process.argv[1];
  const manifest = JSON.parse(fs.readFileSync(path.join(directory, "media-tools.json"), "utf8"));
  const files = { ffmpeg: "ffmpeg", ffprobe: "ffprobe", handbrake: "HandBrakeCLI" };
  for (const [key, filename] of Object.entries(files)) {
    const actual = crypto.createHash("sha256").update(fs.readFileSync(path.join(directory, filename))).digest("hex");
    if (actual !== manifest.tools[key].sha256) {
      throw new Error(`${filename} checksum does not match media-tools.json`);
    }
  }
' "$media_dir"

echo "Verified signed, notarized, architecture-specific macOS application: $app_path"
