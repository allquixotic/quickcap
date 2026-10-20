mod app;
mod audio;
mod capture;
mod media;
mod paths;
mod recording;
mod shell;

use windows::{Win32::UI::WindowsAndMessaging::*, core::PCWSTR};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

pub fn show_error(text: &str) {
    // SAFETY: Both nul-terminated strings live through the synchronous call.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(wide(text).as_ptr()),
            windows::core::w!("quickcap"),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn run() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() > 1 {
        let mode = args[1].to_string_lossy();
        if mode == "--smoke-test" || mode == "--capture-test" {
            let folder = args
                .get(2)
                .ok_or_else(|| anyhow::anyhow!("Expected test output directory"))?;
            let folder = std::path::Path::new(folder);
            std::fs::create_dir_all(folder)?;
            let result = recording::smoke_test(folder, mode == "--capture-test");
            std::fs::write(
                folder.join("result.txt"),
                match &result {
                    Ok(s) => s.clone(),
                    Err(e) => format!("ERROR: {e:#}"),
                },
            )?;
            return result.map(|_| ());
        }
        anyhow::bail!("Unknown argument. Use --smoke-test or --capture-test <directory>.");
    }
    app::run()
}
