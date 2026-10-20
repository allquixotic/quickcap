//! Manual integration test. Run in a disposable Windows desktop session:
//! desktop_verify.exe C:\path\quickcap.exe C:\path\report-directory
//! Sends real input, creates captures, and exits only the quickcap it starts.
#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let folder = std::path::PathBuf::from(&args[2]);
    std::fs::create_dir_all(&folder).unwrap();
    let result = verify::run(std::path::Path::new(&args[1]));
    let report = match &result {
        Ok(report) => report.clone(),
        Err(e) => format!("FAIL: {e:#}"),
    };
    std::fs::write(folder.join("desktop-result.txt"), &report).unwrap();
    println!("{report}");
    if result.is_err() {
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod verify {
    use anyhow::{Context, Result, ensure};
    use quickcap::config::{Binding, Config};
    use std::{
        path::{Path, PathBuf},
        process::{Child, Command},
        thread,
        time::{Duration, Instant},
    };
    use windows::{
        Win32::{
            Foundation::*,
            System::Com::CoTaskMemFree,
            System::DataExchange::GetClipboardSequenceNumber,
            UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
        },
        core::*,
    };
    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn wait(mut condition: impl FnMut() -> bool, seconds: u64) -> Result<()> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(seconds) {
            if condition() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(40));
        }
        anyhow::bail!("Timed out after {seconds} seconds")
    }
    fn folder(id: &GUID) -> Result<PathBuf> {
        // SAFETY: Shell allocated path is copied and freed with CoTaskMemFree.
        unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
            let s = p.to_string();
            CoTaskMemFree(Some(p.0.cast()));
            Ok(s?.into())
        }
    }
    fn files(folder: &Path, ext: &str) -> usize {
        std::fs::read_dir(folder)
            .map(|it| {
                it.flatten()
                    .filter(|e| {
                        e.path().extension().is_some_and(|s| s == ext)
                            && !e.path().to_string_lossy().contains("partial")
                    })
                    .count()
            })
            .unwrap_or(0)
    }
    fn input(kind: INPUT_TYPE, data: INPUT_0) -> INPUT {
        INPUT {
            r#type: kind,
            Anonymous: data,
        }
    }
    fn key(vk: u16, up: bool) -> INPUT {
        input(
            INPUT_KEYBOARD,
            INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    dwFlags: if up {
                        KEYEVENTF_KEYUP
                    } else {
                        KEYBD_EVENT_FLAGS(0)
                    },
                    ..Default::default()
                },
            },
        )
    }
    fn send(events: &[INPUT]) -> Result<()> {
        // SAFETY: Array contains initialized INPUT unions matching each type tag.
        let sent = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
        ensure!(
            sent == events.len() as u32,
            "SendInput rejected; run at same integrity level"
        );
        Ok(())
    }
    fn hotkey(binding: Binding) -> Result<()> {
        let mut v = Vec::new();
        for (mask, vk) in [(1, 0x12), (2, 0x11), (4, 0x10), (8, 0x5b)] {
            if binding.modifiers & mask != 0 {
                v.push(vk);
            }
        }
        let mut events = v.iter().map(|&vk| key(vk, false)).collect::<Vec<_>>();
        events.push(key(binding.key as u16, false));
        events.push(key(binding.key as u16, true));
        events.extend(v.iter().rev().map(|&vk| key(vk, true)));
        send(&events)
    }
    fn selection() -> Option<HWND> {
        // SAFETY: FindWindow reads immutable strings, returns borrowed handle.
        unsafe { FindWindowW(w!("quickcap.selection"), None).ok() }
    }
    fn tray_window() -> Option<HWND> {
        // SAFETY: The constant class string is valid for this read-only lookup.
        unsafe { FindWindowW(w!("quickcap.tray"), None).ok() }
    }
    fn clipboard_sequence() -> u32 {
        // SAFETY: This read-only OS query has no pointer parameters.
        unsafe { GetClipboardSequenceNumber() }
    }
    fn controls_window(pid: u32) -> Option<HWND> {
        let mut after = None;
        // SAFETY: Only accept visible windows owned by the child, not an
        // Explorer folder that happens to have the same quickcap title.
        unsafe {
            while let Ok(hwnd) = FindWindowExW(None, after, None, w!("quickcap")) {
                let mut owner = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut owner));
                if owner == pid && IsWindowVisible(hwnd).as_bool() {
                    return Some(hwnd);
                }
                after = Some(hwnd);
            }
        }
        None
    }
    fn status(hwnd: HWND) -> String {
        let mut buf = [0u16; 2048];
        // SAFETY: GetWindowTextW bounds its write to this initialized slice.
        let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
        String::from_utf16_lossy(&buf[..len.max(0) as usize])
    }
    fn drag() -> Result<()> {
        wait(|| selection().is_some(), 5).context("Selection overlay did not appear")?;
        // SAFETY: Window is live; cursor positions are inside its client area.
        unsafe {
            let mut r = RECT::default();
            GetWindowRect(selection().unwrap(), &mut r)?;
            SetCursorPos(r.left + 100, r.top + 120)?;
            send(&[input(
                INPUT_MOUSE,
                INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: MOUSEEVENTF_LEFTDOWN,
                        ..Default::default()
                    },
                },
            )])?;
            thread::sleep(Duration::from_millis(100));
            SetCursorPos(r.left + 800, r.top + 620)?;
            thread::sleep(Duration::from_millis(100));
            send(&[input(
                INPUT_MOUSE,
                INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: MOUSEEVENTF_LEFTUP,
                        ..Default::default()
                    },
                },
            )])?;
        }
        wait(|| selection().is_none(), 5).context("Selection overlay did not close after drag")
    }
    pub fn run(exe: &Path) -> Result<String> {
        ensure!(
            tray_window().is_none(),
            "An existing quickcap is running; exit it first"
        );
        let log_path = folder(&FOLDERID_Documents)?.join("quickcap/quickcap_log.txt");
        let old_log = if log_path.exists() {
            std::fs::read_to_string(&log_path)?
        } else {
            String::new()
        };
        let started = Instant::now();
        let mut process = Process(Command::new(exe).spawn()?);
        let mut hwnd = None;
        wait(
            || {
                hwnd = tray_window();
                hwnd.is_some()
            },
            20,
        )?;
        let startup = started.elapsed();
        let handle = hwnd.unwrap();
        wait(|| status(handle).starts_with("Ready"), 5).context("Initial ready state")?;
        let config =
            Config::load(&folder(&FOLDERID_Profile)?.join(".config/quickcap/config.json"))?;
        let bindings = config.bindings()?;
        let captures = folder(&FOLDERID_Documents)?.join("quickcap");
        let jpg = files(&captures, "jpg");
        let mp4 = files(&captures, "mp4");
        let seq = clipboard_sequence();
        hotkey(bindings[0])?;
        wait(|| clipboard_sequence() != seq, 5).context("Desktop clipboard")?;
        wait(|| status(handle).starts_with("Image copied"), 5)?;
        println!("PASS desktop clipboard");
        hotkey(bindings[1])?;
        wait(|| files(&captures, "jpg") == jpg + 1, 5).context("Desktop JPEG")?;
        wait(|| status(handle).starts_with("Saved"), 5)?;
        println!("PASS desktop JPEG");
        let seq = clipboard_sequence();
        hotkey(bindings[2])?;
        drag()?;
        wait(|| clipboard_sequence() != seq, 5).context("Region clipboard")?;
        wait(|| status(handle).starts_with("Image copied"), 5)?;
        println!("PASS region clipboard");
        hotkey(bindings[3])?;
        drag()?;
        wait(|| files(&captures, "jpg") == jpg + 2, 5).context("Region JPEG")?;
        wait(|| status(handle).starts_with("Saved"), 5)?;
        println!("PASS region JPEG");
        hotkey(bindings[4])?;
        thread::sleep(Duration::from_millis(400));
        hotkey(bindings[5])?;
        thread::sleep(Duration::from_secs(4));
        ensure!(
            files(&captures, "mp4") == mp4,
            "Cancelled countdown created MP4"
        );
        println!("PASS countdown cancellation");
        hotkey(bindings[4])?;
        thread::sleep(Duration::from_secs(6));
        hotkey(bindings[5])?;
        wait(|| files(&captures, "mp4") == mp4 + 1, 20).context("Desktop recording stop/save")?;
        wait(|| status(handle).starts_with("Saved"), 5)?;
        println!("PASS desktop recording");
        hotkey(bindings[5])?;
        drag()?;
        thread::sleep(Duration::from_secs(6));
        hotkey(bindings[4])?;
        wait(|| files(&captures, "mp4") == mp4 + 2, 20).context("Region recording stop/save")?;
        wait(|| status(handle).starts_with("Saved"), 5)?;
        println!("PASS region recording");
        // Cancel one selection using the other video hotkey, then open another.
        hotkey(bindings[5])?;
        wait(|| selection().is_some(), 5)?;
        hotkey(bindings[4])?;
        wait(|| selection().is_none(), 5)?;
        thread::sleep(Duration::from_millis(200));
        hotkey(bindings[2])?;
        wait(|| selection().is_some(), 5)?;
        send(&[key(0x1b, false), key(0x1b, true)])?;
        wait(|| selection().is_none(), 5)?;
        // Show controls, then request the same graceful exit as the tray menu.
        // SAFETY: Only the window belonging to the process launched here is used.
        unsafe {
            PostMessageW(hwnd, WM_COMMAND, WPARAM(100), LPARAM(0))?;
        }
        let mut controls = None;
        wait(
            || {
                controls = controls_window(process.0.id());
                controls.is_some()
            },
            5,
        )
        .context("Slint control window did not appear")?;
        let controls = controls.unwrap();
        // SAFETY: Only the control window launched by this test is closed.
        unsafe {
            PostMessageW(Some(controls), WM_CLOSE, WPARAM(0), LPARAM(0))?;
        }
        wait(
            || {
                // SAFETY: Windows validates this borrowed window handle.
                !unsafe { IsWindowVisible(controls) }.as_bool()
            },
            5,
        )
        .context("Closing controls should hide them")?;
        ensure!(
            tray_window().is_some(),
            "Closing controls removed tray process"
        );
        // SAFETY: This is still the same test-owned window handle.
        unsafe {
            PostMessageW(hwnd, WM_COMMAND, WPARAM(105), LPARAM(0))?;
        }
        wait(|| process.0.try_wait().ok().flatten().is_some(), 10)?;
        ensure!(process.0.wait()?.success(), "App exited with failure");
        let saved_log = std::fs::read_to_string(&log_path)?;
        ensure!(
            saved_log.starts_with(&old_log),
            "Log overwrote an earlier session"
        );
        let pid = format!("[pid={}]", process.0.id());
        let events: Vec<_> = saved_log
            .lines()
            .filter(|line| line.contains(&pid))
            .collect();
        for required in [
            "started",
            "Ready; config:",
            "Image copied to clipboard",
            "Saved",
            "Recording started:",
            "Recording countdown cancelled",
            "Recording stop requested",
            "Exit requested",
            "Stopped",
        ] {
            ensure!(
                events.iter().any(|line| line.contains(required)),
                "Missing log event: {required}"
            );
        }
        ensure!(
            events.len() < 40,
            "Log is unexpectedly noisy: {} events",
            events.len()
        );
        let mut rejected = Command::new(exe).arg("--invalid-test-argument").spawn()?;
        ensure!(!rejected.wait()?.success(), "Invalid argument should fail");
        let error_log = std::fs::read_to_string(&log_path)?;
        ensure!(
            error_log.starts_with(&saved_log),
            "Next launch overwrote the log"
        );
        let error_pid = format!("[pid={}]", rejected.id());
        ensure!(
            error_log.lines().any(|line| line.contains(&error_pid)
                && line.contains("[ERROR]")
                && line.contains("Fatal error: Unknown argument")),
            "Fatal startup error missing from log"
        );
        println!("PASS persistent, concise log and fatal startup error");
        Ok(format!(
            "PASS\nTray startup: {:.0} ms\nAll six hotkeys; both clipboard paths; desktop/region JPEG; countdown cancel; desktop/region MP4; cross-hotkey stop/save; selection cancellation/restart; Slint window close-to-tray; graceful exit; append-only concise log; fatal startup error logging.\nCaptures: {}",
            startup.as_secs_f64() * 1000.0,
            captures.display()
        ))
    }
}
