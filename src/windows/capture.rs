use crate::{geometry::Rect, pixels::Frame};
use anyhow::{Context, Result, ensure};
use std::{
    ffi::c_void,
    mem::size_of,
    path::{Path, PathBuf},
};
use windows::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{DataExchange::*, Memory::*},
    UI::WindowsAndMessaging::*,
};

pub fn desktop() -> Result<Rect> {
    // SAFETY: GetSystemMetrics reads process DPI-aware desktop metrics.
    let r = unsafe {
        Rect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN) as u32,
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN) as u32,
        }
    };
    r.output_size()?;
    Ok(r)
}

pub fn bitmap_info(w: u32, h: u32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Owns GDI resources on one capture thread. The DIB is always top-down BGRA.
pub struct Grabber {
    screen: HDC,
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut c_void,
    region: Rect,
    size: (u32, u32),
}
impl Grabber {
    pub fn new(region: Rect, full_resolution: bool) -> Result<Self> {
        let size = if full_resolution {
            (region.width, region.height)
        } else {
            region.output_size()?
        };
        ensure!(
            size.0 > 0 && size.1 > 0 && u64::from(size.0) * u64::from(size.1) <= 150_000_000,
            "Desktop capture too large"
        );
        // SAFETY: Acquired DCs/bitmap have unique ownership; every failure path
        // releases earlier resources. DIB allocation size follows validated bounds.
        unsafe {
            let screen = GetDC(None);
            ensure!(!screen.is_invalid(), "GetDC failed");
            let dc = CreateCompatibleDC(Some(screen));
            if dc.is_invalid() {
                ReleaseDC(None, screen);
                anyhow::bail!("CreateCompatibleDC failed");
            }
            let mut bits = std::ptr::null_mut();
            let bitmap = match CreateDIBSection(
                Some(screen),
                &bitmap_info(size.0, size.1),
                DIB_RGB_COLORS,
                &mut bits,
                None,
                0,
            ) {
                Ok(b) => b,
                Err(e) => {
                    let _ = DeleteDC(dc);
                    ReleaseDC(None, screen);
                    return Err(e.into());
                }
            };
            let old = SelectObject(dc, bitmap.into());
            SetStretchBltMode(dc, HALFTONE);
            let _ = SetBrushOrgEx(dc, 0, 0, None);
            Ok(Self {
                screen,
                dc,
                bitmap,
                old,
                bits,
                region,
                size,
            })
        }
    }
    pub fn grab(&self, cursor: bool) -> Result<Frame> {
        // SAFETY: All handles and DIB memory remain owned and live. GDI completes
        // before the buffer is copied. No references to DIB memory escape.
        unsafe {
            StretchBlt(
                self.dc,
                0,
                0,
                self.size.0 as i32,
                self.size.1 as i32,
                Some(self.screen),
                self.region.x,
                self.region.y,
                self.region.width as i32,
                self.region.height as i32,
                SRCCOPY | CAPTUREBLT,
            )
            .ok()
            .context("Desktop capture failed")?;
            if cursor {
                let mut ci = CURSORINFO {
                    cbSize: size_of::<CURSORINFO>() as u32,
                    ..Default::default()
                };
                if GetCursorInfo(&mut ci).is_ok() && ci.flags == CURSOR_SHOWING {
                    let mut info = ICONINFO::default();
                    if GetIconInfo(HICON(ci.hCursor.0), &mut info).is_ok() {
                        let sx = self.size.0 as f64 / self.region.width as f64;
                        let sy = self.size.1 as f64 / self.region.height as f64;
                        let x = ((ci.ptScreenPos.x - self.region.x - info.xHotspot as i32) as f64
                            * sx) as i32;
                        let y = ((ci.ptScreenPos.y - self.region.y - info.yHotspot as i32) as f64
                            * sy) as i32;
                        let _ = DrawIconEx(
                            self.dc,
                            x,
                            y,
                            HICON(ci.hCursor.0),
                            (GetSystemMetrics(SM_CXCURSOR) as f64 * sx).round() as i32,
                            (GetSystemMetrics(SM_CYCURSOR) as f64 * sy).round() as i32,
                            0,
                            None,
                            DI_NORMAL,
                        );
                        if !info.hbmColor.is_invalid() {
                            let _ = DeleteObject(info.hbmColor.into());
                        }
                        if !info.hbmMask.is_invalid() {
                            let _ = DeleteObject(info.hbmMask.into());
                        }
                    }
                }
            }
            let _ = GdiFlush();
            let bgra = std::slice::from_raw_parts(
                self.bits.cast::<u8>(),
                self.size.0 as usize * self.size.1 as usize * 4,
            )
            .to_vec();
            Ok(Frame {
                width: self.size.0,
                height: self.size.1,
                bgra,
            })
        }
    }
}
impl Drop for Grabber {
    fn drop(&mut self) {
        // SAFETY: Resources were acquired by this Grabber; unselect before deletion.
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
            ReleaseDC(None, self.screen);
        }
    }
}

pub fn jpeg(frame: &Frame, folder: &Path) -> Result<PathBuf> {
    let (path, file) = super::paths::reserve(folder, "jpg")?;
    let encoder = jpeg_encoder::Encoder::new(file, 75);
    encoder.encode(
        &frame.bgra,
        frame.width as u16,
        frame.height as u16,
        jpeg_encoder::ColorType::Bgra,
    )?;
    Ok(path)
}

/// Publish a conventional CF_DIB for interoperability with Office applications.
/// Clipboard bitmap formats are uncompressed by Windows design.
pub fn clipboard(frame: &Frame) -> Result<()> {
    let bytes = size_of::<BITMAPINFOHEADER>() + frame.bgra.len();
    // SAFETY: Movable global memory is exclusively owned until SetClipboardData
    // succeeds, when Windows takes ownership. Header and pixels fit allocation.
    unsafe {
        // EmptyClipboard requires a non-null owner for SetClipboardData. Keep a
        // tiny hidden window alive until Windows has accepted the copied DIB.
        let owner = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::w!("STATIC"),
            windows::core::w!("quickcap clipboard"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        )?;
        struct Owner(HWND);
        impl Drop for Owner {
            fn drop(&mut self) {
                // SAFETY: This guard uniquely owns the thread's hidden window.
                unsafe {
                    let _ = DestroyWindow(self.0);
                }
            }
        }
        let _owner = Owner(owner);
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
        let ptr = GlobalLock(memory);
        if ptr.is_null() {
            let _ = GlobalFree(Some(memory));
            anyhow::bail!("GlobalLock failed");
        }
        std::ptr::copy_nonoverlapping(
            (&bitmap_info(frame.width, frame.height).bmiHeader as *const BITMAPINFOHEADER)
                .cast::<u8>(),
            ptr.cast::<u8>(),
            size_of::<BITMAPINFOHEADER>(),
        );
        std::ptr::copy_nonoverlapping(
            frame.bgra.as_ptr(),
            ptr.cast::<u8>().add(size_of::<BITMAPINFOHEADER>()),
            frame.bgra.len(),
        );
        let _ = GlobalUnlock(memory);
        // Clipboard can be briefly held by another process. Retry for 250 ms.
        let mut opened = false;
        for _ in 0..25 {
            if OpenClipboard(Some(owner)).is_ok() {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if !opened {
            let _ = GlobalFree(Some(memory));
            anyhow::bail!("Clipboard is busy");
        }
        let result =
            EmptyClipboard().and_then(|()| SetClipboardData(8, Some(HANDLE(memory.0))).map(|_| ()));
        let _ = CloseClipboard();
        if result.is_err() {
            let _ = GlobalFree(Some(memory));
        }
        result.context("Write image to clipboard")
    }
}
