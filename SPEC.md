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
- I.hotkeys: Ctrl+Alt+Shift+1/2 = desktop clipboard/file; 3/4 = region clipboard/file; 5/6 = desktop/region video. Ctrl+Alt+Shift applies to all six number-row keys. Desktop = entire virtual desktop.
- I.config: `%USERPROFILE%/.config/quickcap/config.json`; validated JSON, editable hotkeys; tray reload.
- I.files: Windows Documents known folder `/quickcap`; timestamped collision-safe `.jpg`/`.mp4`; `.partial.mp4` until successful finalization.
- I.tray: resident icon, red recording indicator; hover/left click show Slint hotkey controls. Right menu: Exit, About, Help; Help = left click. Explorer restart recovery.
- I.ui: Slint controls show active hotkeys; config button reveals config.json in Explorer. Hover preview keeps keyboard focus, dismisses after pointer leaves icon and controls; click/Help stays open. About shows version/project/license. Native physical-pixel region selection, Escape cancels; 3/2/1 countdown after selection; either video key cancels countdown or stops+saves recording.
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
V11: Tray hover never steals focus or interrupts selection/countdown; preview remains clickable. Left click/Help show current active bindings; config opens Explorer with config.json selected; closing help/About leaves tray running.
V12: Controls and About use a consistent light palette; enabled buttons remain readable with Windows dark mode. Shell paths use native separators.

## §T TASKS
id|status|task|cites
---|---|---|---
T1|x|Config, geometry, timeline/state core + named unit tests|V1,V2,V3,V5
T2|x|GDI capture, clipboard/JPEG, WASAPI mix, software MP4 + smoke test|V1,V4,V5,V6,V7,I.test
T3|x|Tray/hotkeys, region selection, Slint controls, countdown + shutdown|V2,V3,V6,V7,V9,I.ui,I.tray
T4|x|CI/signing/docs; Windows build, media decode + desktop validation|V4,V7,V8,I.release
T5|x|Use Ctrl+Alt+Shift+1–6 defaults; update docs and verify bindings|V2,I.hotkeys,I.config
T6|x|Tray hover/click hotkey popup, Explorer config reveal, Exit/About/Help menu|V2,V3,V6,V7,V11,V12,I.tray,I.ui,I.config

## §B BUGS
id|date|cause|fix
---|---|---|---
B1|2026-10-20|Win32 binding signatures and packed audio fields required exact pointer/alignment handling|V7; Windows cross-check
B2|2026-10-20|Null clipboard owner blocks publication; cancelled selection could join replacement worker|V9; native integration coverage
B3|2026-10-20|Hand-built MP3 media type omitted required codec extra data|Use advertised encoder type; native smoke test
B4|2026-10-20|Variable PCM chunk sizes made Windows MP3 encoder emit overlapping timestamps|V10; fixed MPEG frame submissions and independent full decode
B5|2026-10-20|Mixed-separator config path rejected by Shell parser; dark widget palette on hardcoded light window made buttons unreadable|V12; native path components and explicit light palette; Explorer selection test and visual review
B6|2026-10-20|Hover test teleported cursor and could match Explorer's quickcap window; hidden icon bounds pointed to overflow chevron|V11; real SendInput movement, child-process window lookup, open overflow and re-query icon bounds
