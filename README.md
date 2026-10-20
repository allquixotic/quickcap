# quickcap

A small, portable screenshot and screen recording tool for **Windows 11 24H2
(build 26100) or newer, x64 clients**. Run `quickcap.exe`; it stays in the tray.
No installer, administrator rights, GPU, background service, codec download,
FFmpeg, or separately installed runtime is required.

The UI uses Slint's software renderer. Capture uses GDI. Encoding uses Windows'
software Media Foundation H.264 and MP3 encoders with hardware transforms disabled.
All application code is Rust; unsafe code is confined to documented Win32/COM
boundaries. Windows N editions need the Windows Media Feature Pack; Windows
Server and ARM64 are outside the support target.

## Controls

| Default hotkey | Action |
| --- | --- |
| Ctrl+Alt+Shift+1 | Entire virtual desktop → clipboard |
| Ctrl+Alt+Shift+2 | Entire virtual desktop → JPEG file |
| Ctrl+Alt+Shift+3 | Drag a region → clipboard |
| Ctrl+Alt+Shift+4 | Drag a region → JPEG file |
| Ctrl+Alt+Shift+5 | Record entire virtual desktop |
| Ctrl+Alt+Shift+6 | Drag a region, then record it |

The numeric shortcuts use the number row.

Recording starts after a **3, 2, 1** countdown. Press **either video hotkey**
during countdown to cancel, or while recording to **stop and save**. Escape
cancels region selection. The tray icon turns red during recording.

Hover over the tray icon to see the current hotkeys in a Slint popup without
taking keyboard focus. Move into the popup to use its buttons; it closes after
the pointer leaves both the icon and popup. Clicking the popup keeps it open.
Hover previews are suppressed during region selection, countdown, and saving.
Left-click the icon to keep the same controls open. **Show config.json in File
Explorer** opens its containing folder with the file selected. Edit the file,
then click **Reload hotkeys** to apply and display the new bindings.

Right-click the icon for **Exit**, **About**, and **Help**. Help opens the same
window as left-click; About shows the version, license, and project address.
Closing either Slint window keeps the process running. Exit from the tray or
control window to stop it; an active recording is finalized before exit.

These defaults avoid Windows Snipping Tool and SnagIt's default shortcuts, but
another application can still reserve them. A registration conflict is reported.
Only one quickcap process can run per Windows session.

Files go into **Documents\quickcap**, using the Windows Documents known folder
(including redirected/OneDrive Documents), with timestamped, collision-safe
names. MP4 files retain `.partial.mp4` until encoding has successfully finalized.
A crash or power loss can leave an incomplete file; it is retained, not reported
as a saved recording. The clipboard gets a standard Windows bitmap that Office
applications can paste; JPEG is the only image file format quickcap writes.

## Output and audio

- Aspect-preserving downscale to fit 1920×1080; smaller displays are never upscaled.
- JPEG quality 75. Video: H.264 Baseline, 10 fps, nominal 1.5 Mbps.
- Audio: MP3, 48 kHz mono, 64 kbps. Expected total video size is roughly 12 MB/min
  at the nominal bit rate; actual size varies with content and the encoder.
- WASAPI captures the default playback endpoint through loopback and the default
  microphone, mixing both. Windows microphone privacy settings must permit desktop
  applications. Missing, denied, disconnected, or unsupported endpoints become
  silence, with a warning in the completion status.
- Device choices are made when recording starts. Changing defaults during a
  recording does not switch devices; stop and restart to select new defaults.
- Protected video, secure desktops/UAC prompts, and other applications' excluded
  windows cannot necessarily be captured. Headless/disconnected desktops can
  return a black image; use a signed-in, available desktop session.

These settings target ordinary GUI demonstrations and readable text at small
file sizes, not fast motion. Multiple large monitors share one 1080p bounding
box, so select the relevant region when small text matters. Slow machines may
skip frames; timestamps retain real elapsed time. The capture queue and audio
queues are bounded, and the UI and hotkeys run independently of encoders.

## Configuration

On first launch quickcap creates:
`%USERPROFILE%\.config\quickcap\config.json`.

```json
{
  "hotkeys": {
    "desktop_clipboard": "Ctrl+Alt+Shift+1",
    "desktop_file": "Ctrl+Alt+Shift+2",
    "region_clipboard": "Ctrl+Alt+Shift+3",
    "region_file": "Ctrl+Alt+Shift+4",
    "desktop_video": "Ctrl+Alt+Shift+5",
    "region_video": "Ctrl+Alt+Shift+6"
  }
}
```

Edit the file, then choose **Reload hotkeys**. Keys may be A–Z, 0–9, or F1–F24;
modifiers are Ctrl, Alt, Shift, and Win. At least Ctrl, Alt, or Win is required.
Duplicates and unknown fields are errors. Invalid files are never silently
replaced with defaults. A failed registration restores the prior active bindings.
Upgrades preserve existing hotkeys. To adopt these defaults with an older config,
update its six hotkey values to match the example, then reload.

## Build and verify

Install current stable Rust with the MSVC toolchain and Visual Studio C++ build
tools / Windows SDK on a development machine, then:

```text
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release --target x86_64-pc-windows-msvc
```

The executable is `target\x86_64-pc-windows-msvc\release\quickcap.exe`.
The repository statically links the C runtime. Build dependencies are not runtime
dependencies. `rust-toolchain.toml` follows stable; `Cargo.lock` pins crate versions.

On macOS/Linux, core unit tests run with `cargo test --lib`. With `cargo-xwin`
installed, cross-build with `cargo xwin build --release --target
x86_64-pc-windows-msvc`. Only Windows x64 is a supported application target.

Two diagnostic modes bypass the tray and write `result.txt` plus uniquely named
media into the supplied directory:

```text
quickcap.exe --smoke-test C:\Temp\quickcap-smoke
quickcap.exe --capture-test C:\Temp\quickcap-live
```

The smoke test generates three seconds of synthetic video and a 440 Hz tone.
The live test captures the actual desktop and both audio endpoints for four
seconds. Run it inside a normal signed-in desktop session. These modes return
nonzero on errors and never open modal error dialogs. FFprobe/FFmpeg can be used
as independent development checks; they are not used by quickcap itself.

`cargo build --release --example desktop_verify --target x86_64-pc-windows-msvc`
builds the manual desktop interaction test. Run `desktop_verify.exe
C:\path\quickcap.exe C:\Temp\quickcap-interaction` in an otherwise idle desktop
session; it sends real keyboard/mouse input, creates captures, tests all six
hotkeys, and exits the instance it launched. Do not run other capture tests at
the same time. See [verification notes](docs/verification.md) for tested behavior
and remaining validation limits.

The `tray_verify` example tests hover focus/dismissal, left-click help, the config
button's Explorer selection, and the Exit/About/Help menu. Run `tray_verify.exe
C:\path\quickcap.exe C:\Temp\quickcap-tray` in the same way; it also saves JPEG
previews of Help and About. Run interaction tests one at a time.

See [release setup](docs/releasing.md) for Azure signing and
[architecture](docs/architecture.md) for threading, resource ownership, and limits.

## License

GPL-3.0-only; see [LICENSE](LICENSE). Slint is used under its GPL-3.0 option.
Windows components remain part of Windows and are not redistributed. Dependency
licenses are recorded in package metadata and `Cargo.lock`.
