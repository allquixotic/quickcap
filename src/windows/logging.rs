use anyhow::{Context, Result};
use log::{Level, LevelFilter, Log, Metadata, Record};
use std::{
    fmt,
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use windows::Win32::System::SystemInformation::GetLocalTime;

// No buffered writer: each infrequent event reaches the OS before returning,
// including the final panic record before release builds abort.
struct FileLog<W> {
    writer: Mutex<W>,
    failed: AtomicBool,
    report_error: fn(&str),
}

impl<W: Write + Send> FileLog<W> {
    fn new(writer: W, report_error: fn(&str)) -> Self {
        Self {
            writer: Mutex::new(writer),
            failed: AtomicBool::new(false),
            report_error,
        }
    }

    fn check(&self, result: std::io::Result<()>) {
        if let Err(error) = result
            && !self.failed.swap(true, Ordering::Relaxed)
        {
            (self.report_error)(&format!(
                "Cannot write Documents\\quickcap\\quickcap_log.txt: {error}. \
                 Further log write errors will not be shown this session."
            ));
        }
    }
}

struct SingleLine<'a>(&'a fmt::Arguments<'a>);
impl fmt::Display for SingleLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Error chains can contain newlines; keep each event on one physical line.
        for ch in self.0.to_string().chars() {
            write!(f, "{}", if ch.is_control() { ' ' } else { ch })?;
        }
        Ok(())
    }
}

impl<W: Write + Send> Log for FileLog<W> {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Warn
            || (metadata.level() == Level::Info
                && (metadata.target() == "quickcap" || metadata.target().starts_with("quickcap::")))
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // SAFETY: GetLocalTime returns an owned value; no pointers or affinity.
        let t = unsafe { GetLocalTime() };
        let line = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{:<5}] [pid={}] {}: {}\r\n",
            t.wYear,
            t.wMonth,
            t.wDay,
            t.wHour,
            t.wMinute,
            t.wSecond,
            t.wMilliseconds,
            record.level(),
            std::process::id(),
            record.target(),
            SingleLine(record.args())
        );
        let result = self
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .write_all(line.as_bytes());
        // Release the lock before reporting; the reporter can display a dialog.
        self.check(result);
    }

    fn flush(&self) {
        let result = self
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .flush();
        self.check(result);
    }
}

pub fn init(path: &Path) -> Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("Open diagnostic log {}", path.display()))?;
    log::set_boxed_logger(Box::new(FileLog::new(file, super::show_error)))
        .context("Initialize diagnostic logger")?;
    log::set_max_level(LevelFilter::Info);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("Panic: {info}");
        log::logger().flush();
        previous(info);
    }));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io, sync::atomic::AtomicUsize, thread, time::SystemTime};

    fn entry(logger: &impl Log, level: Level, target: &str, text: &str) {
        logger.log(
            &Record::builder()
                .level(level)
                .target(target)
                .args(format_args!("{text}"))
                .build(),
        );
    }
    fn unexpected_error(text: &str) {
        panic!("{text}");
    }

    #[test]
    fn v13_log_appends_filters_and_keeps_concurrent_events_intact() {
        let path = std::env::temp_dir().join(format!(
            "quickcap-log-test-{}-{}.txt",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, "previous session\r\n").unwrap();
        let logger = FileLog::new(
            OpenOptions::new().append(true).open(&path).unwrap(),
            unexpected_error,
        );
        entry(&logger, Level::Info, "winit", "omit dependency info");
        entry(&logger, Level::Debug, "quickcap", "omit debug");
        entry(&logger, Level::Info, "quickcap::windows::app", "started");
        entry(&logger, Level::Warn, "winit", "dependency warning");
        entry(
            &logger,
            Level::Error,
            "quickcap",
            "error chain\nsecond line\r\nthird line",
        );
        thread::scope(|scope| {
            for worker in 0..8 {
                let logger = &logger;
                scope.spawn(move || {
                    for event in 0..40 {
                        entry(
                            logger,
                            Level::Warn,
                            "quickcap",
                            &format!("worker={worker} event={event}"),
                        );
                    }
                });
            }
        });
        logger.flush();
        drop(logger);
        let text = fs::read_to_string(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert!(text.starts_with("previous session\r\n"));
        assert_eq!(text.lines().count(), 324);
        assert!(!text.contains("omit"));
        assert!(text.contains("[INFO ]") && text.contains("[WARN ]") && text.contains("[ERROR]"));
        assert!(text.contains("error chain second line  third line"));
        for line in text.lines().skip(1) {
            assert_eq!(&line[4..5], "-");
            assert_eq!(&line[10..11], " ");
            assert_eq!(&line[19..20], ".");
            assert!(line.contains(&format!("[pid={}]", std::process::id())));
        }
        for worker in 0..8 {
            for event in 0..40 {
                assert_eq!(
                    text.lines()
                        .filter(|line| line.ends_with(&format!("worker={worker} event={event}")))
                        .count(),
                    1
                );
            }
        }
    }

    #[test]
    fn v13_log_reports_write_failure_only_once() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("disk full"))
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::Error::other("disk full"))
            }
        }
        static REPORTED: AtomicUsize = AtomicUsize::new(0);
        let logger = FileLog::new(Broken, |error| {
            assert!(error.contains("disk full"));
            REPORTED.fetch_add(1, Ordering::Relaxed);
        });
        entry(&logger, Level::Error, "quickcap", "first");
        entry(&logger, Level::Error, "quickcap", "second");
        logger.flush();
        assert_eq!(REPORTED.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn v13_panic_hook_records_worker_panic() {
        const CHILD_LOG: &str = "QUICKCAP_PANIC_TEST_LOG";
        if let Some(path) = std::env::var_os(CHILD_LOG) {
            init(Path::new(&path)).unwrap();
            let _ = thread::spawn(|| panic!("intentional logger verification panic")).join();
            std::process::exit(17);
        }
        // A subprocess isolates the global logger and panic hook from other tests,
        // and closes the log handle even when the child aborts.
        let path = std::env::temp_dir().join(format!(
            "quickcap-panic-test-{}-{}.txt",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "windows::logging::tests::v13_panic_hook_records_worker_panic",
            ])
            .env(CHILD_LOG, &path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let text = fs::read_to_string(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert!(text.contains("[ERROR]"));
        assert!(text.contains("Panic:") && text.contains("intentional logger verification panic"));
    }
}
