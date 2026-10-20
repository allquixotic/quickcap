# Verification

Validated on 2026-10-20 using stable Rust 1.99.1, targeting
`x86_64-pc-windows-msvc`. The Windows client was Windows 11 24H2, build 26100.9711,
on the `games` VM. Runtime tests ran through `asuser` as
`AzureAD\SeanMcNamara`; `whoami /groups` confirmed **Medium Mandatory Level**
(`S-1-16-8192`), not an elevated process.

Completed checks:

- 12 portable unit tests on macOS; 13 tests on Windows including native software
  H.264 + MP3 encoding and MP4 finalization.
- Rust formatting and Windows `cargo clippy --all-targets -- -D warnings`.
- GitHub workflow syntax with `actionlint`.
- Synthetic MP4 independently probed as H.264 Constrained Baseline, 640×360,
  10 fps, and MP3 mono at 48 kHz. Synthetic and final live recordings both decoded
  through FFmpeg without errors, including monotonic audio timestamps.
- Live desktop capture at 1920×1080, JPEG output and native WASAPI acquisition of
  both default endpoints without reported errors. Frame drops varied from zero
  to 21 in four-second VM runs; saved timestamps retain elapsed time. This
  validates acquisition, not a separate microphone speech
  quality or long-duration drift measurement.
- `examples/desktop_verify.rs` sent real keyboard/mouse input in the desktop
  session: all six hotkeys, desktop and region clipboard/JPEG, cancellation during
  countdown, desktop and region MP4, stopping with the other video hotkey,
  cancellation and restart of selection, opening and closing Slint controls to
  the tray, graceful exit.
- Observed tray readiness: approximately 44–150 ms in these VM runs; not a guaranteed
  startup bound across machines.
- PE imports contain only Windows DLLs. No D3D/Vulkan/OpenGL import, FFmpeg, codec
  DLL, or dynamically linked MSVC runtime. The build embeds an `asInvoker`
  manifest and statically links the C runtime.

The test binaries and generated captures are local verification artifacts, not
repository inputs or runtime dependencies. Production only needs `quickcap.exe`.

Not verified here: Azure credentials/account setup and actual signing, GitHub
hosted execution, prolonged recordings with clock drift or hotplug, protected
desktops, mixed-DPI negative-origin hardware, and Windows N after Media Feature
Pack installation. Geometry edge cases are unit tested; the VM has a conventional
single virtual desktop. The release workflow requires successful signing and
verification before publishing.
