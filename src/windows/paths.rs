use anyhow::{Context, Result};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};
use windows::{
    Win32::{
        System::{Com::CoTaskMemFree, SystemInformation::GetLocalTime},
        UI::Shell::*,
    },
    core::GUID,
};

fn known_folder(id: &GUID) -> Result<PathBuf> {
    // SAFETY: Shell returns a CoTaskMem-allocated, nul-terminated string. Copy it
    // before freeing its allocation with the matching allocator.
    unsafe {
        let ptr = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None)?;
        let result = ptr.to_string();
        CoTaskMemFree(Some(ptr.0.cast()));
        Ok(PathBuf::from(result?))
    }
}
pub fn output_dir() -> Result<PathBuf> {
    let path = known_folder(&FOLDERID_Documents)?.join("quickcap");
    fs::create_dir_all(&path).context("Create Documents/quickcap")?;
    Ok(path)
}
pub fn config_file() -> Result<PathBuf> {
    Ok(known_folder(&FOLDERID_Profile)?
        .join(".config")
        .join("quickcap")
        .join("config.json"))
}

pub fn reserve(folder: &Path, extension: &str) -> Result<(PathBuf, File)> {
    static SEQUENCE: AtomicU32 = AtomicU32::new(0);
    // SAFETY: GetLocalTime has no caller-owned memory or thread affinity.
    let t = unsafe { GetLocalTime() };
    for _ in 0..1000 {
        let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "quickcap-{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}-{seq}.{extension}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
        );
        let path = folder.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    anyhow::bail!("Unable to reserve a unique output filename")
}
