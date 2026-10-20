# Verification

Validated on 2026-10-20 using stable Rust 1.99.1, targeting
`x86_64-pc-windows-msvc`. The Windows client was Windows 11 24H2, build 26100.9711,
on the `games` VM. Runtime tests ran through `asuser` as
`AzureAD\SeanMcNamara`; `whoami /groups` confirmed **Medium Mandatory Level**
(`S-1-16-8192`), not an elevated process.

Completed checks:

- 12 portable unit tests on macOS; 16 tests on Windows including native software
  H.264 + MP3 encoding, MP4 finalization, and file logging.
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
- Repeated the full desktop interaction test after switching the defaults to
  Ctrl+Alt+Shift+1–6. A fresh config contained the number-row bindings, and all
  six shortcuts and cross-hotkey stop/save passed.
- `examples/tray_verify.rs` exercised native mouse hover over the actual tray
  icon, opening Windows' hidden-icon panel when necessary. Hover preserved
  foreground focus, remained visible at the popup's bottom edge, and dismissed
  after the pointer left. Left-click help stayed open.
- The config button opened File Explorer; Shell COM confirmed that the exact
  `config.json` file was selected. Mouse clicks exercised all three native menu
  items: Help, About, and Exit. Opening the menu hid Slint windows so they could
  not cover it. Closing Help/About retained the tray process.
- Help and About JPEG previews were visually checked on dark-mode Windows.
  Both use a consistent light palette with readable buttons. All 13 Windows
  tests and the full capture interaction test passed again after the UI changes.
- Logging tests verified preservation of earlier sessions, 320 concurrent
  complete messages, severity filtering, single-line error formatting, reporting
  a failed writer only once, and worker panic logging in an isolated subprocess.
  The full desktop and tray interaction tests passed with logging enabled;
  the desktop test checked capture/lifecycle events, bounded log volume, append
  behavior on a second launch, and an actual fatal invalid-argument error.
  Native WASAPI discontinuity warnings were present in the resulting log.
- Observed tray readiness: approximately 43–155 ms in these VM runs; not a guaranteed
  startup bound across machines.
- PE imports contain only Windows DLLs. No D3D/Vulkan/OpenGL import, FFmpeg, codec
  DLL, or dynamically linked MSVC runtime. The build embeds an `asInvoker`
  manifest and statically links the C runtime.
- [First hosted Windows CI](https://github.com/allquixotic/quickcap/actions/runs/39462224748)
  passed formatting, Clippy, all 16 native tests, release compilation, and the
  executable's synthetic media test. The tag workflow repeated those checks.
- [Signed v0.1.0 release](https://github.com/allquixotic/quickcap/releases/tag/v0.1.0)
  was built and published by [GitHub Actions](https://github.com/allquixotic/quickcap/actions/runs/39464360232).
  Azure OIDC authentication, signing, expected publisher/timestamp verification,
  signed-executable smoke test, and provenance generation all passed.
- Independently downloaded the published executable, checked its SHA-256 against
  `SHA256SUMS`, and verified GitHub attestation with the exact release workflow,
  tag ref, and hosted-runner requirement. Windows 11 reported a valid signature
  from Sean McNamara with a Microsoft timestamp. The full desktop interaction
  test passed on that signed executable, including all six hotkeys, recording,
  clipboard, shutdown, and persistent logging. PE imports still require only
  Windows system DLLs.

Published executable: 8,682,760 bytes. SHA-256:
`7108526d697793feb049786d3f950793d01abfc469030cd7178493cbb2f41369`.

The test binaries and generated captures are local verification artifacts, not
repository inputs or runtime dependencies. Production only needs `quickcap.exe`.

Not verified here: prolonged recordings with clock drift or hotplug, protected
desktops, mixed-DPI negative-origin hardware, and Windows N after Media Feature
Pack installation. Geometry edge cases are unit tested; the VM has a conventional
single virtual desktop. The release workflow requires successful signing and
verification before publishing.
