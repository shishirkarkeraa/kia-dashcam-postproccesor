# Third-party media tools

Kia Dashcam Processor invokes the executables below from its own private
`media-tools` directory. They are packaged with the application and standalone CLI;
the application does not locate them on `PATH` and does not download them at runtime.
Versions, immutable download locations, and archive SHA-256 values are recorded in
`media-tools.lock.json`.

## FFmpeg and FFprobe

- Project and corresponding source code: <https://ffmpeg.org/>
- macOS and Linux packaged-build source: <https://ffmpeg.martin-riedl.de/>
- Windows packaged-build source: <https://github.com/BtbN/FFmpeg-Builds>
- License: GNU General Public License version 3 or later for these GPL-enabled builds
- License and redistribution information: <https://ffmpeg.org/legal.html>
- License text: <https://github.com/FFmpeg/FFmpeg/blob/master/COPYING.GPLv3>

The staging gate executes both programs with `-L`, rejects any build marked
non-redistributable or configured with `--enable-nonfree`, and requires the GPL and
libx264 features used by the unchanged processing pipeline.

## HandBrakeCLI

- Project and corresponding source code: <https://github.com/HandBrake/HandBrake>
- Pinned version: 1.11.2 (`9eb6c936803e8b071035b1a77662cb0db58441ea`)
- License: GNU General Public License version 2
- License text: <https://github.com/HandBrake/HandBrake/blob/1.11.2/COPYING>

The Rust/Tauri application communicates with these programs only through their
public command-line interfaces. No media-tool source code is linked into the Rust
application.
