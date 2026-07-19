# Staged application media tools

Run `npm run prepare:media --prefix apps/dashcam-gui` from the repository root.
The staging gate downloads the platform's pinned archives, verifies every SHA-256,
rejects non-redistributable FFmpeg configurations, and writes the private
`media-tools` resource directory used by Tauri. Downloaded executables are ignored by
Git and must never be committed.
