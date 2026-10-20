use super::{capture, wide};
use crate::{config::Binding, geometry::Rect, pixels::Frame};
use anyhow::{Context, Result, ensure};
use std::{
    cell::{Cell, RefCell},
    mem::size_of,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{LibraryLoader::GetModuleHandleW, Threading::*},
        UI::{Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
    },
    core::*,
};

const WAKE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;
const ICON_ID: u32 = 1;
const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
pub const SHOW: u32 = 100;
pub const FOLDER: u32 = 101;
pub const CONFIG: u32 = 102;
pub const RELOAD: u32 = 103;
pub const STOP: u32 = 104;
pub const QUIT: u32 = 105;
pub const ABOUT: u32 = 106;

#[derive(Clone, Debug)]
pub enum Event {
    Hotkey(usize),
    Menu(u32),
    Error(String),
    Reloaded,
    Hover(bool),
    MenuOpening,
}
enum Command {
    Status(bool, String),
    Reload(Vec<Binding>),
    Quit,
}
pub type Notify = Arc<dyn Fn(Event) + Send + Sync>;
pub struct Shell {
    hwnd: usize,
    tx: mpsc::Sender<Command>,
    worker: Option<JoinHandle<()>>,
}
impl Shell {
    pub fn start(bindings: Vec<Binding>, notify: Notify) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let report = notify.clone();
            if let Err(e) = shell_loop(bindings, rx, notify, ready_tx.clone()) {
                report(Event::Error(format!("Tray thread failed: {e:#}")));
                let _ = ready_tx.send(Err(e));
            }
        });
        let hwnd = ready_rx.recv().context("Tray thread failed")??;
        Ok(Self {
            hwnd,
            tx,
            worker: Some(worker),
        })
    }
    fn send(&self, command: Command) {
        let _ = self.tx.send(command);
        // SAFETY: Posting to a window handle is thread-safe; destruction races
        // merely return an error, which is ignored during shutdown.
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as *mut _)), WAKE, WPARAM(0), LPARAM(0));
        }
    }
    pub fn status(&self, recording: bool, text: &str) {
        self.send(Command::Status(recording, text.into()));
    }
    pub fn reload(&self, bindings: Vec<Binding>) {
        self.send(Command::Reload(bindings));
    }
    pub fn pointer_over_icon(&self) -> bool {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: HWND(self.hwnd as *mut _),
            uID: ICON_ID,
            ..Default::default()
        };
        // SAFETY: Read-only queries; the returned rectangle is owned locally.
        unsafe {
            Shell_NotifyIconGetRect(&identifier).is_ok_and(|rect| {
                pointer_inside(
                    rect.left,
                    rect.top,
                    (rect.right - rect.left).max(0) as u32,
                    (rect.bottom - rect.top).max(0) as u32,
                )
            })
        }
    }
    pub fn popup_position(&self, width: u32, height: u32) -> Option<(i32, i32)> {
        let identifier = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: HWND(self.hwnd as *mut _),
            uID: ICON_ID,
            ..Default::default()
        };
        // SAFETY: All structures are initialized; Shell resolves our live icon.
        // Windows constrains the popup to the icon's monitor work area.
        unsafe {
            let icon = Shell_NotifyIconGetRect(&identifier).ok()?;
            let anchor = POINT {
                x: icon.right,
                y: icon.top,
            };
            let size = SIZE {
                cx: width as i32,
                cy: height as i32,
            };
            let mut position = RECT::default();
            CalculatePopupWindowPosition(
                &anchor,
                &size,
                (TPM_RIGHTALIGN | TPM_BOTTOMALIGN | TPM_WORKAREA).0,
                Some(&icon),
                &mut position,
            )
            .ok()?;
            Some((position.left, position.top))
        }
    }
}
impl Drop for Shell {
    fn drop(&mut self) {
        self.send(Command::Quit);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct State {
    rx: mpsc::Receiver<Command>,
    notify: Notify,
    bindings: Vec<Binding>,
    normal: HICON,
    recording: HICON,
    is_recording: bool,
    status: String,
    taskbar_created: u32,
    tray_failed: Cell<bool>,
}
thread_local! {static STATE:RefCell<Option<State>>=const{RefCell::new(None)};}

fn register(hwnd: HWND, bindings: &[Binding]) -> Result<()> {
    // SAFETY: Called only on the owning message thread. Roll back partial sets.
    unsafe {
        for (i, b) in bindings.iter().enumerate() {
            if let Err(e) = RegisterHotKey(
                Some(hwnd),
                i as i32,
                HOT_KEY_MODIFIERS(b.modifiers) | MOD_NOREPEAT,
                b.key,
            ) {
                for j in 0..i {
                    let _ = UnregisterHotKey(Some(hwnd), j as i32);
                }
                return Err(e).context(format!(
                    "Hotkey {} is already in use; edit config.json and restart",
                    i + 1
                ));
            }
        }
    }
    Ok(())
}

fn icon(recording: bool) -> Result<HICON> {
    let mut pixels = vec![0u8; 32 * 32 * 4];
    let mut mask = [0xffu8; 128];
    for y in 0..32 {
        for x in 0..32 {
            let inside = if recording {
                (x as i32 - 16).pow(2) + (y as i32 - 16).pow(2) <= 12 * 12
            } else {
                (4..28).contains(&x) && (6..26).contains(&y)
            };
            if inside {
                let p = &mut pixels[(y * 32 + x) * 4..][..4];
                p.copy_from_slice(if recording {
                    &[30, 30, 230, 255]
                } else {
                    &[215, 125, 35, 255]
                });
                mask[y * 4 + x / 8] &= !(0x80 >> (x % 8));
            }
        }
    }
    // SAFETY: Buffers cover the full 32×32 XOR bitmap and 1-bit AND mask.
    unsafe {
        Ok(CreateIcon(
            None,
            32,
            32,
            1,
            32,
            mask.as_ptr(),
            pixels.as_ptr(),
        )?)
    }
}

fn tray(hwnd: HWND, state: &State, operation: NOTIFY_ICON_MESSAGE) {
    let mut data = NOTIFYICONDATAW {
        cbSize: size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: TRAY,
        hIcon: if state.is_recording {
            state.recording
        } else {
            state.normal
        },
        ..Default::default()
    };
    for (dst, src) in data
        .szTip
        .iter_mut()
        .take(127)
        .zip(state.status.encode_utf16())
    {
        *dst = src;
    }
    // SAFETY: Shell copies the initialized structure synchronously; icons remain live.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide(&state.status).as_ptr()));
        if !Shell_NotifyIconW(operation, &data).as_bool() {
            if !state.tray_failed.replace(true) {
                log::error!(
                    "Windows could not update the tray icon (operation {})",
                    operation.0
                );
            }
        } else if state.tray_failed.replace(false) {
            log::info!("Tray icon recovered");
        }
        if operation == NIM_ADD {
            // Version 4 supplies native hover notifications and keyboard activation.
            // It must be restored after Explorer recreates the icon as well.
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            if !Shell_NotifyIconW(NIM_SETVERSION, &data).as_bool() {
                log::warn!("Windows could not enable version-4 tray notifications");
            }
        }
    }
}

fn shell_loop(
    bindings: Vec<Binding>,
    rx: mpsc::Receiver<Command>,
    notify: Notify,
    ready: mpsc::SyncSender<Result<usize>>,
) -> Result<()> {
    // SAFETY: A top-level hidden window receives Explorer broadcast messages.
    // The class callback accesses thread-local state only on this message thread.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = w!("quickcap.tray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module.into(),
            lpszClassName: class,
            ..Default::default()
        };
        ensure!(RegisterClassW(&wc) != 0, "Register tray class failed");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            w!("quickcap"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(module.into()),
            None,
        )?;
        let init = (|| -> Result<State> {
            register(hwnd, &bindings)?;
            Ok(State {
                rx,
                notify,
                bindings,
                normal: icon(false)?,
                recording: icon(true)?,
                is_recording: false,
                status: "quickcap — ready".into(),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                tray_failed: Cell::new(false),
            })
        })();
        let state = match init {
            Ok(s) => s,
            Err(e) => {
                let _ = ready.send(Err(e));
                let _ = DestroyWindow(hwnd);
                return Ok(());
            }
        };
        tray(hwnd, &state, NIM_ADD);
        STATE.with(|slot| *slot.borrow_mut() = Some(state));
        let _ = ready.send(Ok(hwnd.0 as usize));
        let mut msg = MSG::default();
        loop {
            let result = GetMessageW(&mut msg, None, 0, 0).0;
            if result <= 0 {
                if result < 0 {
                    log::error!("Tray message loop failed: {}", Error::from_thread());
                }
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        STATE.with(|slot| {
            if let Some(s) = slot.borrow_mut().take() {
                tray(hwnd, &s, NIM_DELETE);
                let _ = DestroyIcon(s.normal);
                let _ = DestroyIcon(s.recording);
            }
        });
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let handled = STATE.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return false;
        };
        let Some(state) = slot.as_mut() else {
            return false;
        };
        // SAFETY: Called by Windows for this thread's window. Reentrant callbacks
        // fall through when the RefCell is borrowed, avoiding aliased mutable state.
        unsafe {
            if msg == state.taskbar_created {
                log::info!("Explorer restarted; restoring tray icon");
                tray(hwnd, state, NIM_ADD);
                return true;
            }
            match msg {
                WM_COMMAND => {
                    (state.notify)(Event::Menu((wparam.0 & 0xffff) as u32));
                    true
                }
                WM_HOTKEY => {
                    (state.notify)(Event::Hotkey(wparam.0));
                    true
                }
                TRAY => {
                    // Version 4 packs the icon ID in HIWORD and event in LOWORD.
                    if ((lparam.0 as u32 >> 16) & 0xffff) != ICON_ID {
                        return true;
                    }
                    match lparam.0 as u32 & 0xffff {
                        // Mouse movement opens the preview promptly; the native
                        // rich-tooltip event covers keyboard/accessibility hover.
                        // Dismissal checks actual pointer geometry in both cases.
                        NIN_POPUPOPEN | WM_MOUSEMOVE => (state.notify)(Event::Hover(true)),
                        NIN_POPUPCLOSE => (state.notify)(Event::Hover(false)),
                        NIN_SELECT | NIN_KEYSELECT => (state.notify)(Event::Menu(SHOW)),
                        WM_CONTEXTMENU => {
                            (state.notify)(Event::MenuOpening);
                            if let Ok(menu) = CreatePopupMenu() {
                                for (id, label) in
                                    [(QUIT, "Exit"), (ABOUT, "About"), (SHOW, "Help")]
                                {
                                    let _ = AppendMenuW(
                                        menu,
                                        MF_STRING,
                                        id as usize,
                                        PCWSTR(wide(label).as_ptr()),
                                    );
                                }
                                let mut p = POINT::default();
                                let _ = GetCursorPos(&mut p);
                                let _ = SetForegroundWindow(hwnd);
                                let selected = TrackPopupMenu(
                                    menu,
                                    TPM_RETURNCMD | TPM_NONOTIFY,
                                    p.x,
                                    p.y,
                                    None,
                                    hwnd,
                                    None,
                                )
                                .0;
                                let _ = DestroyMenu(menu);
                                if selected != 0 {
                                    (state.notify)(Event::Menu(selected as u32));
                                }
                                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                            } else {
                                log::error!(
                                    "Could not create tray context menu: {}",
                                    Error::from_thread()
                                );
                            }
                        }
                        _ => {}
                    }
                    true
                }
                WAKE => {
                    while let Ok(command) = state.rx.try_recv() {
                        match command {
                            Command::Status(recording, text) => {
                                state.is_recording = recording;
                                state.status = text;
                                tray(hwnd, state, NIM_MODIFY);
                            }
                            Command::Reload(bindings) => {
                                for i in 0..6 {
                                    let _ = UnregisterHotKey(Some(hwnd), i);
                                }
                                match register(hwnd, &bindings) {
                                    Ok(()) => {
                                        state.bindings = bindings;
                                        (state.notify)(Event::Reloaded);
                                    }
                                    Err(e) => {
                                        let restore = register(hwnd, &state.bindings);
                                        (state.notify)(Event::Error(format!(
                                            "{e:#}{}",
                                            restore
                                                .err()
                                                .map(|e| format!(
                                                    "; restoring old hotkeys also failed: {e:#}"
                                                ))
                                                .unwrap_or_default()
                                        )));
                                    }
                                }
                            }
                            Command::Quit => {
                                for i in 0..6 {
                                    let _ = UnregisterHotKey(Some(hwnd), i);
                                }
                                PostQuitMessage(0);
                            }
                        }
                    }
                    true
                }
                _ => false,
            }
        }
    });
    if handled {
        LRESULT(0)
    } else {
        // SAFETY: Forwarding an unhandled message with original parameters.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

pub fn pointer_inside(x: i32, y: i32, width: u32, height: u32) -> bool {
    let mut point = POINT::default();
    // SAFETY: GetCursorPos writes one initialized POINT; no retained pointers.
    (unsafe { GetCursorPos(&mut point).is_ok() })
        && point.x >= x
        && point.y >= y
        && i64::from(point.x) < i64::from(x) + i64::from(width)
        && i64::from(point.y) < i64::from(y) + i64::from(height)
}

/// Reveal an absolute filesystem path without command-line quoting or file associations.
/// Called on a worker so starting Explorer cannot stall the UI thread.
pub fn reveal(path: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::*;
    // Rust accepts mixed separators, but the Shell namespace parser does not.
    let native_path: std::path::PathBuf = path.components().collect();
    let path: Vec<u16> = native_path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY: This worker owns its COM apartment. The Shell-allocated PIDL is
    // freed after the synchronous call, and every successful init is balanced.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .context("Initialize Explorer worker")?;
        let result = (|| -> Result<()> {
            let mut item = std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(path.as_ptr()), None, &mut item, 0, None)
                .with_context(|| format!("Resolve {} in Explorer", native_path.display()))?;
            let result = SHOpenFolderAndSelectItems(item, None, 0);
            CoTaskMemFree(Some(item.cast()));
            result.context("Could not reveal config.json in Explorer")
        })();
        CoUninitialize();
        result
    }
}

pub fn open(path: &std::path::Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: Shell copies this nul-terminated path during the call.
    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    ensure!(result.0 as isize > 32, "Could not open file or folder");
    Ok(())
}

struct Selection {
    background: Frame,
    desktop: Rect,
    start: Option<(i32, i32)>,
    current: (i32, i32),
    result: Option<Rect>,
}
thread_local! {static SELECTION:RefCell<Option<Selection>>=const{RefCell::new(None)};}

pub fn select(cancel: Arc<AtomicBool>) -> Result<Option<Rect>> {
    let desktop = capture::desktop()?;
    let background = capture::Grabber::new(desktop, true)?.grab(false)?;
    // SAFETY: Selection window and state are confined to this dedicated thread.
    unsafe {
        let module = GetModuleHandleW(None)?;
        let class = w!("quickcap.selection");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(selection_proc),
            hInstance: module.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_CROSS)?,
            ..Default::default()
        };
        RegisterClassW(&wc); // Class can already exist after a previous selection.
        SELECTION.with(|s| {
            *s.borrow_mut() = Some(Selection {
                background,
                desktop,
                start: None,
                current: (0, 0),
                result: None,
            })
        });
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class,
            w!("quickcap — drag to select; Esc cancels"),
            WS_POPUP | WS_VISIBLE,
            desktop.x,
            desktop.y,
            desktop.width as i32,
            desktop.height as i32,
            None,
            None,
            Some(module.into()),
            None,
        )?;
        let _ = SetForegroundWindow(hwnd);
        SetFocus(Some(hwnd))?;
        SetTimer(Some(hwnd), 1, 30, None);
        let mut msg = MSG::default();
        loop {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            let result = GetMessageW(&mut msg, None, 0, 0).0;
            if result <= 0 {
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = KillTimer(Some(hwnd), 1);
        let _ = DestroyWindow(hwnd);
        Ok(SELECTION.with(|s| s.borrow_mut().take().and_then(|s| s.result)))
    }
}

unsafe extern "system" fn selection_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let handled = SELECTION.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return false;
        };
        let Some(s) = slot.as_mut() else {
            return false;
        };
        let point = (
            (lparam.0 as i16 as i32).clamp(0, s.desktop.width as i32),
            ((lparam.0 >> 16) as i16 as i32).clamp(0, s.desktop.height as i32),
        );
        // SAFETY: Valid window/DC handles; paint data lives throughout Begin/EndPaint.
        unsafe {
            match msg {
                WM_LBUTTONDOWN => {
                    s.start = Some(point);
                    s.current = point;
                    SetCapture(hwnd);
                    true
                }
                WM_MOUSEMOVE => {
                    s.current = point;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    true
                }
                WM_LBUTTONUP => {
                    if let Some(start) = s.start {
                        s.result = Rect::from_points(
                            (start.0 + s.desktop.x, start.1 + s.desktop.y),
                            (point.0 + s.desktop.x, point.1 + s.desktop.y),
                        );
                    }
                    let _ = ReleaseCapture();
                    PostQuitMessage(0);
                    true
                }
                WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
                    PostQuitMessage(0);
                    true
                }
                WM_TIMER => true,
                WM_ERASEBKGND => true,
                WM_PAINT => {
                    let mut ps = PAINTSTRUCT::default();
                    let dc = BeginPaint(hwnd, &mut ps);
                    let frame = &s.background;
                    SetDIBitsToDevice(
                        dc,
                        0,
                        0,
                        frame.width,
                        frame.height,
                        0,
                        0,
                        0,
                        frame.height,
                        frame.bgra.as_ptr().cast(),
                        &capture::bitmap_info(frame.width, frame.height),
                        DIB_RGB_COLORS,
                    );
                    SetBkColor(dc, COLORREF(0x202020));
                    SetTextColor(dc, COLORREF(0xffffff));
                    let text = wide("Drag a box to capture. Esc cancels.");
                    let _ = TextOutW(dc, 20, 20, &text[..text.len() - 1]);
                    if let Some(start) = s.start {
                        let pen = CreatePen(PS_SOLID, 3, COLORREF(0x0020ff));
                        let old_pen = SelectObject(dc, pen.into());
                        let old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
                        let _ = Rectangle(
                            dc,
                            start.0.min(s.current.0),
                            start.1.min(s.current.1),
                            start.0.max(s.current.0),
                            start.1.max(s.current.1),
                        );
                        SelectObject(dc, old_pen);
                        SelectObject(dc, old_brush);
                        let _ = DeleteObject(pen.into());
                    }
                    let _ = EndPaint(hwnd, &ps);
                    true
                }
                _ => false,
            }
        }
    });
    if handled {
        LRESULT(0)
    } else {
        // SAFETY: Original callback parameters are forwarded unchanged.
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }
}

pub struct Instance(HANDLE);
impl Instance {
    pub fn acquire() -> Result<Self> {
        // SAFETY: Named mutex is used only as an instance marker, never locked.
        unsafe {
            let h = CreateMutexW(None, false, w!("Local\\quickcap.instance"))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(h);
                anyhow::bail!("quickcap is already running; use its tray icon");
            }
            Ok(Self(h))
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: This guard uniquely owns the kernel handle.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
