//! Interactive Windows tray acceptance test; run only in a disposable desktop.
//! tray_verify.exe C:\path\quickcap.exe C:\path\report-directory
#[cfg(windows)]
use quickcap::{geometry, pixels};
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/windows/capture.rs"]
mod capture;
#[cfg(windows)]
#[allow(dead_code)]
#[path = "../src/windows/paths.rs"]
mod paths;

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let output = std::path::Path::new(&args[2]);
    std::fs::create_dir_all(output).unwrap();
    let result = verify::run(std::path::Path::new(&args[1]), output);
    let report = match &result {
        Ok(()) => "PASS: hover focus/dismissal, clickable preview, left click, Explorer selection, Help, About, Exit".into(),
        Err(e) => format!("FAIL: {e:#}"),
    };
    std::fs::write(output.join("tray-result.txt"), &report).unwrap();
    println!("{report}");
    if result.is_err() {
        std::process::exit(1);
    }
}

#[cfg(windows)]
mod verify {
    use anyhow::{Context, Result, ensure};
    use std::{
        path::Path,
        process::{Child, Command},
        sync::atomic::{AtomicU32, Ordering},
        thread,
        time::{Duration, Instant},
    };
    static CHILD_PID: AtomicU32 = AtomicU32::new(0);
    use windows::{
        Win32::{
            Foundation::*,
            System::{Com::*, Variant::VARIANT},
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
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: Balanced with the successful initialization in run.
            unsafe {
                CoUninitialize();
            }
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
    fn find(title: PCWSTR) -> Option<HWND> {
        let mut after = None;
        // SAFETY: Enumerate matching top-level windows and accept only visible
        // windows from our child, never an Explorer folder also named quickcap.
        unsafe {
            while let Ok(hwnd) = FindWindowExW(None, after, None, title) {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == CHILD_PID.load(Ordering::Relaxed) && IsWindowVisible(hwnd).as_bool() {
                    return Some(hwnd);
                }
                after = Some(hwnd);
            }
        }
        None
    }
    fn title(hwnd: HWND) -> String {
        let mut text = [0u16; 2048];
        // SAFETY: Windows bounds the write to the supplied slice.
        let len = unsafe { GetWindowTextW(hwnd, &mut text) };
        String::from_utf16_lossy(&text[..len.max(0) as usize])
    }
    fn notify(hwnd: HWND, event: u32) -> Result<()> {
        // SAFETY: Target is the tray window of our child; packed version-4 event.
        unsafe {
            PostMessageW(
                Some(hwnd),
                WM_APP + 2,
                WPARAM(0),
                LPARAM(((1 << 16) | event) as isize),
            )?;
        }
        Ok(())
    }
    fn close(hwnd: HWND) -> Result<()> {
        // SAFETY: Only closes this test's child window.
        unsafe {
            PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0))?;
        }
        wait(|| !visible(hwnd), 5)
    }
    fn visible(hwnd: HWND) -> bool {
        // SAFETY: Windows validates the borrowed handle.
        unsafe { IsWindowVisible(hwnd).as_bool() }
    }
    fn cursor(x: i32, y: i32) -> Result<()> {
        // SAFETY: Read-only desktop metrics and one initialized absolute mouse
        // event. SendInput exercises the same hover route as physical movement.
        unsafe {
            let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let width = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
            let height = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
            let input = INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: ((i64::from(x - left) * 65536 + 32768) / i64::from(width))
                            .clamp(0, 65535) as i32,
                        dy: ((i64::from(y - top) * 65536 + 32768) / i64::from(height))
                            .clamp(0, 65535) as i32,
                        dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                        ..Default::default()
                    },
                },
            };
            ensure!(
                SendInput(&[input], std::mem::size_of::<INPUT>() as i32) == 1,
                "Mouse movement failed"
            );
        }
        Ok(())
    }
    fn snapshot(hwnd: HWND, output: &Path, name: &str) -> Result<()> {
        // Visibility precedes Slint's first software-rendered frame.
        thread::sleep(Duration::from_millis(300));
        let mut r = RECT::default();
        // SAFETY: Read-only bounds for our child window.
        unsafe {
            GetWindowRect(hwnd, &mut r)?;
        }
        let frame = crate::capture::Grabber::new(
            crate::geometry::Rect {
                x: r.left,
                y: r.top,
                width: (r.right - r.left) as u32,
                height: (r.bottom - r.top) as u32,
            },
            true,
        )?
        .grab(false)?;
        let path = crate::capture::jpeg(&frame, output)?;
        std::fs::rename(path, output.join(name))?;
        Ok(())
    }
    fn menu(hwnd: HWND, item: i32, output: &Path) -> Result<()> {
        cursor(400, 400)?;
        notify(hwnd, WM_CONTEXTMENU)?;
        let mut menu = None;
        wait(
            || {
                // SAFETY: Read-only lookup of the native popup menu.
                menu = unsafe {
                    FindWindowW(w!("#32768"), None)
                        .ok()
                        .filter(|h| IsWindowVisible(*h).as_bool())
                };
                menu.is_some()
            },
            5,
        )
        .context("Context menu did not open")?;
        wait(
            || find(w!("quickcap")).is_none() && find(w!("About quickcap")).is_none(),
            5,
        )
        .context("Right-click menu should not be covered by Slint windows")?;
        let mut bounds = RECT::default();
        // SAFETY: The native popup menu is live; read its screen bounds.
        unsafe {
            GetWindowRect(menu.unwrap(), &mut bounds)?;
        }
        thread::sleep(Duration::from_millis(200));
        snapshot(menu.unwrap(), output, &format!("menu-{item}.jpg"))?;
        let x = (bounds.left + bounds.right) / 2;
        let y = bounds.top + (bounds.bottom - bounds.top) * (item * 2 + 1) / 6;
        click(x, y)
    }
    fn click(x: i32, y: i32) -> Result<()> {
        cursor(x, y)?;
        thread::sleep(Duration::from_millis(150));
        let clicks = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        });
        ensure!(
            // SAFETY: Both INPUT unions contain initialized mouse events.
            unsafe { SendInput(&clicks, std::mem::size_of::<INPUT>() as i32) } == 2,
            "Menu click failed"
        );
        Ok(())
    }
    fn explorer_selected(config: &Path) -> Result<bool> {
        // SAFETY: All COM objects stay in this initialized STA; returned variants,
        // BSTRs and interface references have Rust-owned lifetimes.
        unsafe {
            let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)?;
            for i in 0..shell.Count()? {
                let selected = (|| -> Result<bool> {
                    let browser: IWebBrowser2 = shell.Item(&VARIANT::from(i))?.cast()?;
                    let view: IShellFolderViewDual = browser.Document()?.cast()?;
                    let items = view.SelectedItems()?;
                    for j in 0..items.Count()? {
                        let path = items.Item(&VARIANT::from(j))?.Path()?.to_string();
                        if Path::new(&path) == config {
                            return Ok(true);
                        }
                    }
                    Ok(false)
                })();
                if selected.unwrap_or(false) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
    pub fn run(exe: &Path, output: &Path) -> Result<()> {
        // SAFETY: This test initializes one STA and balances it with a guard.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        let _apartment = Apartment;
        ensure!(
            // SAFETY: Read-only lookup; refusing to touch an existing app instance.
            unsafe { FindWindowW(w!("quickcap.tray"), None).is_err() },
            "quickcap already running"
        );
        let mut process = Process(Command::new(exe).spawn()?);
        CHILD_PID.store(process.0.id(), Ordering::Relaxed);
        let mut tray = None;
        wait(
            || {
                // SAFETY: Borrowed child window handle.
                tray = unsafe { FindWindowW(w!("quickcap.tray"), None).ok() };
                tray.is_some_and(|h| title(h).starts_with("Ready"))
            },
            20,
        )?;
        let tray = tray.unwrap();
        cursor(20, 20)?;
        // SAFETY: Read-only query of current foreground window.
        let mut foreground = unsafe { GetForegroundWindow() };
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: tray,
            uID: 1,
            ..Default::default()
        };
        // SAFETY: Read-only geometry of our live notification icon.
        let rect = unsafe { Shell_NotifyIconGetRect(&identifier)? };
        cursor((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)?;
        thread::sleep(Duration::from_millis(900));
        if find(w!("quickcap")).is_none() {
            // A hidden icon's rectangle points to the overflow chevron. Open
            // that panel, then require a distinct rectangle for the actual icon.
            click((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2)?;
            let mut icon = rect;
            wait(
                || {
                    // SAFETY: Re-query after Explorer exposes the icon in overflow.
                    icon = unsafe { Shell_NotifyIconGetRect(&identifier) }.unwrap_or(rect);
                    icon.left != rect.left || icon.top != rect.top
                },
                5,
            )
            .context("Tray icon was not exposed by overflow")?;
            thread::sleep(Duration::from_millis(500));
            // SAFETY: Re-query after the overflow panel's opening animation.
            icon = unsafe { Shell_NotifyIconGetRect(&identifier)? };
            // SAFETY: Opening overflow can legitimately change the foreground.
            foreground = unsafe { GetForegroundWindow() };
            cursor((icon.left + icon.right) / 2, (icon.top + icon.bottom) / 2)?;
        }
        thread::sleep(Duration::from_millis(500));
        wait(|| find(w!("quickcap")).is_some(), 5).context("Hover did not show help")?;
        let controls = find(w!("quickcap")).unwrap();
        thread::sleep(Duration::from_millis(500));
        ensure!(
            // SAFETY: Same read-only query after opening the hover preview.
            unsafe { GetForegroundWindow() } == foreground,
            "Hover stole keyboard focus"
        );
        snapshot(controls, output, "help.jpg")?;
        let mut r = RECT::default();
        // SAFETY: Read-only geometry of the child controls.
        unsafe {
            GetWindowRect(controls, &mut r)?;
        }
        cursor((r.left + r.right) / 2, r.bottom - 8)?;
        notify(tray, NIN_POPUPCLOSE)?;
        thread::sleep(Duration::from_millis(900));
        ensure!(
            visible(controls),
            "Preview closed before pointer could reach controls"
        );
        cursor(20, 20)?;
        wait(|| !visible(controls), 5).context("Hover did not dismiss after leaving controls")?;
        println!("PASS native hover preserves focus, remains interactive, and dismisses");

        notify(tray, NIN_SELECT)?;
        wait(|| find(w!("quickcap")).is_some(), 5)?;
        let controls = find(w!("quickcap")).unwrap();
        notify(tray, NIN_POPUPCLOSE)?;
        thread::sleep(Duration::from_millis(900));
        ensure!(visible(controls), "Left-click help did not remain open");
        let mut client = RECT::default();
        let mut origin = POINT::default();
        // SAFETY: Read-only geometry for our known 540×600 Slint layout.
        unsafe {
            GetClientRect(controls, &mut client)?;
            windows::Win32::Graphics::Gdi::ClientToScreen(controls, &mut origin).ok()?;
        }
        // Center of the first button, below the six hotkey rows.
        click(
            origin.x + client.right / 2,
            origin.y + client.bottom * 32 / 100,
        )?;
        wait(|| title(tray).starts_with("Config file shown"), 15).with_context(|| {
            format!("Config button did not reveal file; status: {}", title(tray))
        })?;
        let config = crate::paths::config_file()?;
        wait(|| explorer_selected(&config).unwrap_or(false), 15)
            .context("Explorer did not select config.json")?;
        println!("PASS left click and config button select config.json in Explorer");

        menu(tray, 2, output)?;
        wait(|| find(w!("quickcap")).is_some(), 5).context("Help menu item")?;
        close(find(w!("quickcap")).unwrap())?;
        menu(tray, 1, output)?;
        wait(|| find(w!("About quickcap")).is_some(), 5).context("About menu item")?;
        let about = find(w!("About quickcap")).unwrap();
        snapshot(about, output, "about.jpg")?;
        close(about)?;
        ensure!(
            process.0.try_wait()?.is_none(),
            "Closing About exited the app"
        );
        println!("PASS Help and About menu items; close keeps tray resident");
        menu(tray, 0, output)?;
        wait(|| process.0.try_wait().ok().flatten().is_some(), 10).context("Exit menu item")?;
        ensure!(process.0.wait()?.success(), "Exit failed");
        Ok(())
    }
}
