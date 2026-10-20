# Architecture

`config`, `geometry`, `pixels`, `state`, and `timeline` are portable, safe Rust.
`windows/` owns all native integration. There is no custom C/C++, bundled media
framework, or network code.

| Thread | Work |
| --- | --- |
| Main | Slint/winit software UI, state transitions, async event dispatch |
| Native shell | Tray messages, hotkey registration, Explorer restart handling |
| Temporary selector | Frozen desktop overlay; physical coordinates and mouse selection |
| Recording | Media Foundation sink writer, interleaving samples, MP4 finalization |
| Capture | GDI downscale, cursor composition, parallel RGB→NV12 conversion |
| Two audio workers | Independent WASAPI loopback and microphone capture |
| Rayon pool | Parallel pixel rows; lazily initialized |

Idle startup does not initialize Media Foundation, WASAPI, the Rayon pool, or
capture buffers. Slint selects `winit` / `software` explicitly, and no other
renderer is compiled. The native hidden window remains independent of the UI,
which can stay closed while hotkeys operate. Thread messages use bounded channels
for frame/audio data, and Slint's event-loop proxy for control events; no async
network runtime is needed.

Capture uses a top-down GDI DIB at final dimensions. `StretchBlt` with HALFTONE
performs the downscale before copying pixels into Rust-owned memory. RGB→NV12 uses
BT.601 limited range, matched by video media-type attributes. A two-frame queue
bounds encoder backlog. Frame timestamps are based on capture time, not the speed
of the encoder; a delayed sample extends the previous frame instead of compressing
elapsed time.

WASAPI uses each endpoint's native mix format. Packet timestamps are converted
from the same QPC timebase used to start video. PCM is downmixed and linearly
resampled to 48 kHz mono. Per-endpoint packet queues are bounded. Mixing leaves
100 ms for audio arrival, drops obsolete packets and saturates to signed 16-bit
PCM. The MP3 encoder receives fixed 1152-sample blocks regardless of polling
cadence; the final block pads at most 1151 samples. Missing devices contribute
silence. Finalization flushes the audio and
video encoder through the sink writer before renaming the file.

Unsafe blocks contain Win32/COM calls or bounded copies from native allocations.
Each block documents ownership and validity. COM initialization is scoped to the
worker thread. GDI objects are restored before deletion. Clipboard memory is
transferred only after a successful `SetClipboardData`. Worker shutdown releases
WASAPI buffers, stops clients, joins capture workers and finalizes the writer.

Known boundaries: the initial version captures the virtual desktop or one region,
not individual windows, and does not migrate a recording when display topology or
default audio devices change. OS capture exclusions apply. A `.partial.mp4` left
by abrupt termination is not guaranteed recoverable. Ten frames/second and mono
audio are deliberate fixed settings, not a general-purpose media editor.
