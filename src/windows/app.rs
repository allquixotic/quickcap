use super::{
    capture, paths, recording,
    shell::{self, Event, Shell},
};
use crate::{
    config::{ACTION_LABELS, Config},
    geometry::Rect,
    state::{Phase, VideoAction},
};
use anyhow::Result;
use slint::ComponentHandle;
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

slint::slint! {
    import { Button, VerticalBox, HorizontalBox } from "std-widgets.slint";
    export component Controls inherits Window {
        title: "quickcap";
        width: 480px;
        height: 530px;
        background: #f3f3f3;
        in property <string> status: "Ready";
        in property <string> bindings;
        in property <bool> busy: false;
        callback action(int);
        VerticalBox {
            padding: 16px;
            spacing: 10px;
            Text { text: "quickcap"; font-size: 24px; font-weight: 700; color: #151515; }
            Text { text: root.status; wrap: word-wrap; color: #151515; min-height: 50px; }
            Text { text: "JPEG screenshots · H.264 + MP3 recordings"; color: #454545; }
            HorizontalBox {
                Button { text: "Desktop → clipboard"; enabled: !root.busy; clicked => { root.action(0); } }
                Button { text: "Desktop → JPEG"; enabled: !root.busy; clicked => { root.action(1); } }
            }
            HorizontalBox {
                Button { text: "Region → clipboard"; enabled: !root.busy; clicked => { root.action(2); } }
                Button { text: "Region → JPEG"; enabled: !root.busy; clicked => { root.action(3); } }
            }
            HorizontalBox {
                Button { text: "Record desktop"; enabled: !root.busy; clicked => { root.action(4); } }
                Button { text: "Record region"; enabled: !root.busy; clicked => { root.action(5); } }
            }
            Button { text: "Stop / cancel recording"; enabled: root.busy; clicked => { root.action(104); } }
            Text { text: root.bindings; font-size: 12px; color: #454545; }
            HorizontalBox {
                Button { text: "Captures"; clicked => { root.action(101); } }
                Button { text: "Config"; clicked => { root.action(102); } }
                Button { text: "Reload"; clicked => { root.action(103); } }
                Button { text: "Exit"; clicked => { root.action(105); } }
            }
            Text { text: "Close this window to keep quickcap in the tray."; color: #454545; font-size: 12px; }
        }
    }
}

enum Message {
    Shell(Event),
    Selected(u64, usize, Result<Option<Rect>>),
    Tick(u64, Rect, u8),
    Start(u64, Rect),
    RecordingStarted,
    Recorded(Result<recording::Outcome>),
    Screenshot(Result<String>),
}
struct App {
    ui: Controls,
    shell: Shell,
    config: Config,
    pending_config: Option<Config>,
    config_path: PathBuf,
    folder: PathBuf,
    phase: Phase,
    generation: u64,
    cancel: Arc<AtomicBool>,
    selection: Option<JoinHandle<()>>,
    recording: Option<JoinHandle<()>>,
    screenshots: Vec<JoinHandle<()>>,
    quitting: bool,
}
thread_local! {static APP:RefCell<Option<Rc<RefCell<App>>>>=const{RefCell::new(None)};}

fn post(message: Message) {
    let _ = slint::invoke_from_event_loop(move || dispatch(message));
}
fn dispatch(message: Message) {
    APP.with(|app| {
        let app = app.borrow().clone();
        if let Some(app) = app {
            let mut app = app.borrow_mut();
            if let Err(e) = app.message(message) {
                app.error(format!("{e:#}"));
            }
        }
    });
}

pub fn run() -> Result<()> {
    let _instance = shell::Instance::acquire()?;
    let config_path = paths::config_file()?;
    let config = Config::load(&config_path)?;
    let folder = paths::output_dir()?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let ui = Controls::new()?;
    ui.on_action(|action| {
        dispatch(Message::Shell(if action < 6 {
            Event::Hotkey(action as usize)
        } else {
            Event::Menu(action as u32)
        }))
    });
    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    let shell = Shell::start(
        config.bindings()?,
        Arc::new(|event| post(Message::Shell(event))),
    )?;
    let app = App {
        ui,
        shell,
        config,
        pending_config: None,
        config_path,
        folder,
        phase: Phase::Idle,
        generation: 0,
        cancel: Arc::new(AtomicBool::new(false)),
        selection: None,
        recording: None,
        screenshots: Vec::new(),
        quitting: false,
    };
    app.update_bindings();
    app.status("Ready — use a hotkey or click the tray icon.");
    APP.with(|slot| *slot.borrow_mut() = Some(Rc::new(RefCell::new(app))));
    slint::run_event_loop_until_quit()?;
    APP.with(|slot| {
        if let Some(app) = slot.borrow_mut().take() {
            app.borrow_mut().shutdown();
        }
    });
    Ok(())
}

impl App {
    fn update_bindings(&self) {
        self.ui.set_bindings(
            self.config
                .strings()
                .into_iter()
                .zip(ACTION_LABELS)
                .map(|(key, label)| format!("{key}  {label}"))
                .collect::<Vec<_>>()
                .join("\n")
                .into(),
        );
    }
    fn status(&self, text: &str) {
        self.ui.set_status(text.into());
        self.shell.status(self.phase == Phase::Recording, text);
        self.ui.set_busy(self.phase != Phase::Idle);
    }
    fn error(&mut self, text: String) {
        self.status(&text);
        let _ = self.ui.show();
    }
    fn idle(&mut self, text: &str) {
        self.phase = Phase::Idle;
        self.status(text);
    }
    fn message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Shell(Event::Hotkey(action)) => self.action(action)?,
            Message::Shell(Event::Error(error)) => {
                self.pending_config = None;
                self.error(error);
            }
            Message::Shell(Event::Reloaded) => {
                if let Some(config) = self.pending_config.take() {
                    self.config = config;
                    self.update_bindings();
                }
                self.status("Hotkeys reloaded");
            }
            Message::Shell(Event::Menu(menu)) => match menu {
                shell::SHOW => self.ui.show()?,
                shell::FOLDER => shell::open(&self.folder)?,
                shell::CONFIG => shell::open(&self.config_path)?,
                shell::RELOAD => {
                    let config = Config::load(&self.config_path)?;
                    self.shell.reload(config.bindings()?);
                    self.pending_config = Some(config);
                    self.status("Hotkey reload requested");
                }
                shell::STOP => self.stop(),
                shell::QUIT => {
                    self.quitting = true;
                    self.stop();
                    if self.recording.is_none() {
                        slint::quit_event_loop()?;
                    }
                }
                _ => {}
            },
            Message::Selected(generation, action, result) => {
                if generation != self.generation {
                    return Ok(());
                }
                if let Some(worker) = self.selection.take() {
                    let _ = worker.join();
                }
                if self.cancel.load(Ordering::Acquire) {
                    self.idle("Selection cancelled");
                    return Ok(());
                }
                match result {
                    Ok(Some(rect)) => {
                        if action >= 4 {
                            self.countdown(rect)?;
                        } else {
                            self.screenshot(rect, action % 2 == 0);
                        }
                    }
                    Ok(None) => self.idle("Selection cancelled"),
                    Err(e) => {
                        self.idle("Selection failed");
                        return Err(e);
                    }
                }
            }
            Message::Tick(generation, rect, seconds)
                if generation == self.generation && self.phase == Phase::Countdown =>
            {
                if seconds == 0 {
                    self.ui.hide()?;
                    self.status("Starting recording…");
                    slint::Timer::single_shot(Duration::from_millis(150), move || {
                        dispatch(Message::Start(generation, rect))
                    });
                } else {
                    self.status(&format!(
                        "Recording starts in {seconds}…\nEither video hotkey cancels."
                    ));
                    slint::Timer::single_shot(Duration::from_secs(1), move || {
                        dispatch(Message::Tick(generation, rect, seconds - 1))
                    });
                }
            }
            Message::Start(generation, rect)
                if generation == self.generation && self.phase == Phase::Countdown =>
            {
                self.record(rect)
            }
            Message::RecordingStarted => {
                if self.phase == Phase::Recording {
                    self.status("Recording — either video hotkey stops and saves");
                }
            }
            Message::Recorded(result) => {
                if let Some(worker) = self.recording.take() {
                    let _ = worker.join();
                }
                self.phase = Phase::Idle;
                match result {
                    Ok(o) => self.status(&format!(
                        "Saved {}{}{}",
                        o.path.display(),
                        if o.warnings.is_empty() {
                            String::new()
                        } else {
                            format!("\n{}", o.warnings.join("; "))
                        },
                        if o.dropped == 0 {
                            String::new()
                        } else {
                            format!("\n{} frames skipped under load", o.dropped)
                        }
                    )),
                    Err(e) => self.error(format!(
                        "Recording failed: {e:#}. Any partial MP4 was retained."
                    )),
                }
                if self.quitting {
                    slint::quit_event_loop()?;
                }
            }
            Message::Screenshot(result) => {
                self.phase = Phase::Idle;
                match result {
                    Ok(text) => self.status(&text),
                    Err(e) => self.error(format!("Screenshot failed: {e:#}")),
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn action(&mut self, action: usize) -> Result<()> {
        if action >= 6 {
            return Ok(());
        }
        if action >= 4 && self.phase != Phase::Idle {
            self.stop();
            return Ok(());
        }
        if self.phase != Phase::Idle {
            return Ok(());
        }
        self.ui.hide()?;
        if action == 2 || action == 3 || action == 5 {
            self.generation += 1;
            let generation = self.generation;
            self.phase = Phase::Selecting;
            self.status("Drag to select a region; Esc cancels");
            self.cancel = Arc::new(AtomicBool::new(false));
            let cancel = self.cancel.clone();
            self.selection = Some(thread::spawn(move || {
                thread::sleep(Duration::from_millis(150));
                post(Message::Selected(generation, action, shell::select(cancel)));
            }));
        } else if action == 4 {
            self.countdown(capture::desktop()?)?;
        } else {
            // Let the compositor remove our controls before capturing pixels.
            let region = capture::desktop()?;
            self.phase = Phase::Selecting;
            self.screenshot(region, action == 0);
        }
        Ok(())
    }
    fn screenshot(&mut self, rect: Rect, clipboard: bool) {
        self.phase = Phase::Selecting;
        self.status("Taking screenshot…");
        let folder = self.folder.clone();
        self.screenshots.retain(|w| !w.is_finished());
        self.screenshots.push(thread::spawn(move || {
            thread::sleep(Duration::from_millis(150));
            post(Message::Screenshot((|| -> Result<String> {
                let frame = capture::Grabber::new(rect, false)?.grab(true)?;
                if clipboard {
                    capture::clipboard(&frame)?;
                    Ok("Image copied to clipboard".into())
                } else {
                    Ok(format!(
                        "Saved {}",
                        capture::jpeg(&frame, &folder)?.display()
                    ))
                }
            })()));
        }));
    }
    fn countdown(&mut self, rect: Rect) -> Result<()> {
        self.phase = Phase::Countdown;
        self.generation += 1;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.ui.show()?;
        let generation = self.generation;
        // The first update is queued to avoid reentering App's RefCell borrow.
        post(Message::Tick(generation, rect, 3));
        Ok(())
    }
    fn record(&mut self, rect: Rect) {
        self.phase = Phase::Recording;
        self.status("Preparing software encoder…");
        let folder = self.folder.clone();
        let stop = self.cancel.clone();
        self.recording = Some(thread::spawn(move || {
            post(Message::Recorded(recording::record(
                rect,
                &folder,
                stop,
                || post(Message::RecordingStarted),
            )))
        }));
    }
    fn stop(&mut self) {
        match self.phase.video_action() {
            VideoAction::StopAndSave => {
                self.cancel.store(true, Ordering::Release);
                self.phase = Phase::Stopping;
                self.status("Saving MP4…");
            }
            VideoAction::CancelCountdown => {
                self.cancel.store(true, Ordering::Release);
                self.generation += 1;
                let _ = self.ui.hide();
                self.idle("Cancelled");
            }
            VideoAction::CancelSelection if self.selection.is_some() => {
                self.cancel.store(true, Ordering::Release);
                self.phase = Phase::Stopping;
                self.status("Cancelling selection…");
            }
            _ => {}
        }
    }
    fn shutdown(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(w) = self.selection.take() {
            let _ = w.join();
        }
        if let Some(w) = self.recording.take() {
            let _ = w.join();
        }
        for w in self.screenshots.drain(..) {
            let _ = w.join();
        }
    }
}
