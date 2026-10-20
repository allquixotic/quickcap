# SPEC

## §G GOAL
Small portable Windows x64 tray tool: JPEG screenshots, H.264 + MP3 MP4 recordings.

## §C CONSTRAINTS
- Rust stable; safe Rust except audited Win32/COM boundaries. Slint software renderer.
- CPU-only capture/encoding/UI; no GPU, admin, drivers, downloads, installer, external codec executable.
- Single EXE, static CRT. Windows 11 24H2 x64 client with built-in Media Foundation; Server out of scope.
- Utilitarian UI; initialize capture/audio/encoders on demand. Worker threads, bounded queues.
- Video defaults: 10 fps, 1.5 Mbps, 1920×1080 bounding box, aspect preserved, no upscale; MP3 mono 48 kHz/64 kbps. JPEG quality 75.
- Capture default output loopback + default microphone, best effort per endpoint; report failures.
- Public source GPL-3.0-only; release pipeline Azure signing, no unsigned release fallback.

## §I INTERFACES
- I.hotkeys: Ctrl+Alt+Shift+F1/F2 = desktop clipboard/file; F3/F4 = region clipboard/file; F5/F6 = desktop/region video. Desktop = entire virtual desktop.
- I.config: `%USERPROFILE%/.config/quickcap/config.json`; validated JSON, editable hotkeys; tray reload.
- I.files: Windows Documents known folder `/quickcap`; timestamped collision-safe `.jpg`/`.mp4`; `.partial.mp4` until successful finalization.
- I.tray: resident icon, red recording indicator, status/open folder/config/reload/stop/quit menu. Explorer restart recovery.
- I.ui: Slint control window; native physical-pixel region selection, Escape cancels; 3/2/1 countdown after selection; either video key cancels countdown or stops+saves recording.
- I.test: `--smoke-test <directory>` records short synthetic H.264+MP3 clip and JPEG without desktop; `--capture-test <directory>` exercises real GDI/WASAPI in desktop session.
- I.release: GitHub Actions build/test; signed `quickcap.exe` + checksums on version tag; OIDC Azure configuration documented.

## §V INVARIANTS
V1: Valid dimensions, aspect retained within even-pixel rounding, no upscale, ≤1920×1080; negative-origin regions supported.
V2: All six hotkeys distinct; malformed/unknown/duplicate bindings rejected without overwriting config.
V3: Only one selection/countdown/recording at once; stop works across either video key; shutdown joins workers and finalizes.
V4: File creation never overwrites; incomplete videos retain `.partial.mp4`; successful MP4 contains only H.264 and MP3.
V5: Audio shares video monotonic clock; absent input/output becomes silence without unbounded buffering; both endpoint failures reported.
V6: GUI and hotkeys remain responsive; bounded capture pipeline drops stale frames without speeding up playback.
V7: No application elevation or GPU API; standard-user native smoke and desktop capture tests succeed.
V8: Release built with stable Rust, static CRT, software-only Slint; signing verified before publication.
V9: Selection cancellation waits for worker handoff before accepting another action; clipboard publication uses non-null window owner.
V10: MP3 PCM submissions ! fixed 1152-sample blocks; final block pads ≤1151 samples. Decoded audio timestamps ! monotonic.

## §T TASKS
id|status|task|cites
---|---|---|---
T1|x|Config, geometry, timeline/state core + named unit tests|V1,V2,V3,V5
T2|x|GDI capture, clipboard/JPEG, WASAPI mix, software MP4 + smoke test|V1,V4,V5,V6,V7,I.test
T3|x|Tray/hotkeys, region selection, Slint controls, countdown + shutdown|V2,V3,V6,V7,V9,I.ui,I.tray
T4|x|CI/signing/docs; Windows build, media decode + desktop validation|V4,V7,V8,I.release

## §B BUGS
id|date|cause|fix
---|---|---|---
B1|2026-10-20|Win32 binding signatures and packed audio fields required exact pointer/alignment handling|V7; Windows cross-check
B2|2026-10-20|Null clipboard owner blocks publication; cancelled selection could join replacement worker|V9; native integration coverage
B3|2026-10-20|Hand-built MP3 media type omitted required codec extra data|Use advertised encoder type; native smoke test
B4|2026-10-20|Variable PCM chunk sizes made Windows MP3 encoder emit overlapping timestamps|V10; fixed MPEG frame submissions and independent full decode
