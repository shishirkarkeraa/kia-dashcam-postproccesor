#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/build-linux-handbrake.sh <output-path>" >&2
  exit 2
fi

readonly repository="https://github.com/HandBrake/HandBrake.git"
readonly tag="1.11.2"
readonly expected_commit="9eb6c936803e8b071035b1a77662cb0db58441ea"
readonly output_path="$1"
readonly work_root="$(mktemp -d)"
readonly source_dir="$work_root/HandBrake"

git clone --filter=blob:none --branch "$tag" --single-branch "$repository" "$source_dir"
readonly actual_commit="$(git -C "$source_dir" rev-parse HEAD)"
if [[ "$actual_commit" != "$expected_commit" ]]; then
  echo "HandBrake source mismatch: expected $expected_commit, got $actual_commit" >&2
  exit 1
fi

cd "$source_dir"
./configure --disable-gtk --launch --launch-jobs="$(getconf _NPROCESSORS_ONLN)"
mkdir -p "$(dirname "$output_path")"
cp build/HandBrakeCLI "$output_path"
chmod 755 "$output_path"
