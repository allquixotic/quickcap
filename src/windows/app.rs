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
use slint::{
    ComponentHandle,
    winit_030::{
        WinitWindowAccessor,
        winit::{event::WindowEvent, window::WindowLevel},
    },
};
use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

slint::slint! {
    import { Button, VerticalBox, HorizontalBox, Palette } from "std-widgets.slint";
    export component Controls inherits Window {
        title: "quickcap";
        width: 540px;
        height: 600px;
        background: #f3f3f3;
        in property <string> status: "Ready";
        in property <string> bindings;
        in property <bool> busy: false;
        callback action(int);
        init => { Palette.color-scheme = ColorScheme.light; }
        VerticalBox {
            padding: 16px;
            spacing: 8px;
            Text { text: "quickcap"; font-size: 24px; font-weight: 700; color: #151515; }
            Text { text: "Current hotkeys"; font-weight: 700; color: #151515; }
            Text { text: root.bindings; font-size: 13px; color: #454545; }
            Button { text: "Show config.json in File Explorer"; clicked => { root.action(102); } }
            HorizontalBox {
                Button { text: "Captures"; clicked => { root.action(101); } }
                Button { text: "Reload hotkeys"; clicked => { root.action(103); } }
                Button { text: "Exit"; clicked => { root.action(105); } }
            }
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
            Text { text: "Close this window to keep quickcap in the tray."; color: #454545; font-size: 12px; }
        }
    }
    export component About inherits Window {
        title: "About quickcap";
        width: 440px;
        height: 230px;
        background: #f3f3f3;
        in property <string> details;
        callback dismiss();
        init => { Palette.color-scheme = ColorScheme.light; }
        VerticalBox {
            padding: 16px;
            spacing: 12px;
            Text { text: "quickcap"; font-size: 24px; font-weight: 700; color: #151515; }
            Text { text: root.details; wrap: word-wrap; color: #151515; }
            Button { text: "Close"; clicked => { root.dismiss(); } }
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
    CheckHover,
    PinControls,
    ConfigRevealed(Result<()>),
}
struct App {
    ui: Controls,
    about: Option<About>,
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
    hover_preview: bool,
    hover_last_inside: Instant,
    hover_timer: slint::Timer,
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
        .with_winit_window_attributes_hook(|attributes| attributes.with_active(false))
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
    ui.window().on_winit_window_event(|_, event| {
        if matches!(event, WindowEvent::Focused(true)) {
            post(Message::PinControls);
        }
        slint::winit_030::EventResult::Propagate
    });
    let shell = Shell::start(
        config.bindings()?,
        Arc::new(|event| post(Message::Shell(event))),
    )?;
    let app = App {
        ui,
        about: None,
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
        hover_preview: false,
        hover_last_inside: Instant::now(),
        hover_timer: slint::Timer::default(),
    };
    app.update_bindings();
    app.status("Ready — use a hotkey or click the tray icon.");
    log::info!(
        "Ready; config: {}; captures: {}",
        app.config_path.display(),
        app.folder.display()
    );
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
    fn pin_controls(&mut self) {
        self.hover_preview = false;
        self.hover_timer.stop();
    }
    fn controls_size(&self) -> (u32, u32) {
        self.ui
            .window()
            .with_winit_window(|window| {
                let size = window.outer_size();
                (size.width, size.height)
            })
            .unwrap_or_else(|| {
                let size = self.ui.window().size();
                (size.width, size.height)
            })
    }
    fn show_controls(&mut self, preview: bool) -> Result<()> {
        if preview && self.ui.window().is_visible() {
            return Ok(());
        }
        if !preview {
            self.pin_controls();
        }
        if !self.ui.window().is_visible() {
            self.hover_preview = preview;
            self.ui.show()?;
            let (width, height) = self.controls_size();
            if let Some((x, y)) = self.shell.popup_position(width, height) {
                self.ui
                    .window()
                    .set_position(slint::PhysicalPosition::new(x, y));
            }
            if preview {
                self.hover_last_inside = Instant::now();
                self.hover_timer.start(
                    slint::TimerMode::Repeated,
                    Duration::from_millis(150),
                    || dispatch(Message::CheckHover),
                );
            }
        }
        self.ui.window().with_winit_window(|window| {
            window.set_window_level(WindowLevel::AlwaysOnTop);
            if !preview {
                window.focus_window();
            }
        });
        Ok(())
    }
    fn check_hover(&mut self) -> Result<()> {
        if !self.hover_preview || !self.ui.window().is_visible() {
            self.pin_controls();
            return Ok(());
        }
        let position = self.ui.window().position();
        let (width, height) = self.controls_size();
        if self.shell.pointer_over_icon()
            || shell::pointer_inside(position.x, position.y, width, height)
        {
            self.hover_last_inside = Instant::now();
        } else if self.hover_last_inside.elapsed() >= Duration::from_millis(600) {
            self.pin_controls();
            self.ui.hide()?;
        }
        Ok(())
    }
    fn show_about(&mut self) -> Result<()> {
        if self.hover_preview {
            self.pin_controls();
            self.ui.hide()?;
        }
        if self.about.is_none() {
            let about = About::new()?;
            about.set_details(
                format!(
                    "Version {}\nJPEG screenshots and H.264 + MP3 recordings\n\nGPL-3.0-only\n{}",
                    env!("CARGO_PKG_VERSION"),
                    env!("CARGO_PKG_REPOSITORY")
                )
                .into(),
            );
            let weak = about.as_weak();
            about.on_dismiss(move || {
                if let Some(about) = weak.upgrade() {
                    let _ = about.hide();
                }
            });
            let weak = about.as_weak();
            about.window().on_close_requested(move || {
                if let Some(about) = weak.upgrade() {
                    let _ = about.hide();
                }
                slint::CloseRequestResponse::KeepWindowShown
            });
            self.about = Some(about);
        }
        let about = self.about.as_ref().unwrap();
        about.show()?;
        about.window().with_winit_window(|window| {
            window.set_window_level(WindowLevel::AlwaysOnTop);
            window.focus_window();
        });
        Ok(())
    }
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
        log::error!("{text}");
        self.show_error(text);
    }
    fn show_error(&mut self, text: String) {
        self.status(&text);
        let _ = self.show_controls(false);
    }
    fn idle(&mut self, text: &str) {
        self.phase = Phase::Idle;
        self.status(text);
    }
    fn message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::PinControls => self.pin_controls(),
            Message::CheckHover => self.check_hover()?,
            Message::ConfigRevealed(result) => {
                result?;
                self.status(
                    "Config file shown in File Explorer. Edit it, then click Reload hotkeys.",
                );
            }
            Message::Shell(Event::Hover(inside)) => {
                if inside && matches!(self.phase, Phase::Idle | Phase::Recording) {
                    self.show_controls(true)?;
                }
            }
            Message::Shell(Event::MenuOpening) => {
                self.pin_controls();
                self.ui.hide()?;
                if let Some(about) = &self.about {
                    about.hide()?;
                }
            }
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
                log::info!("Hotkeys reloaded");
            }
            Message::Shell(Event::Menu(menu)) => match menu {
                shell::SHOW => self.show_controls(false)?,
                shell::ABOUT => self.show_about()?,
                shell::FOLDER => shell::open(&self.folder)?,
                shell::CONFIG => {
                    let path = self.config_path.clone();
                    thread::spawn(move || post(Message::ConfigRevealed(shell::reveal(&path))));
                }
                shell::RELOAD => {
                    let config = Config::load(&self.config_path)?;
                    self.shell.reload(config.bindings()?);
                    self.pending_config = Some(config);
                    self.status("Hotkey reload requested");
                }
                shell::STOP => self.stop(),
                shell::QUIT => {
                    log::info!("Exit requested");
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
                    Err(e) => self.show_error(format!(
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
                    Err(e) => self.show_error(format!("Screenshot failed: {e:#}")),
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
        self.pin_controls();
        if let Some(about) = &self.about {
            about.hide()?;
        }
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
            let result = (|| -> Result<String> {
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
            })();
            match &result {
                Ok(text) => log::info!("{text}"),
                Err(error) => log::error!("Screenshot failed: {error:#}"),
            }
            post(Message::Screenshot(result));
        }));
    }
    fn countdown(&mut self, rect: Rect) -> Result<()> {
        self.phase = Phase::Countdown;
        self.generation += 1;
        self.cancel = Arc::new(AtomicBool::new(false));
        self.show_controls(false)?;
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
            let result = recording::record(rect, &folder, stop, || post(Message::RecordingStarted));
            if let Err(error) = &result {
                log::error!("Recording failed: {error:#}. Any partial MP4 was retained.");
            }
            post(Message::Recorded(result));
        }));
    }
    fn stop(&mut self) {
        match self.phase.video_action() {
            VideoAction::StopAndSave => {
                log::info!("Recording stop requested; saving MP4");
                self.cancel.store(true, Ordering::Release);
                self.phase = Phase::Stopping;
                self.status("Saving MP4…");
            }
            VideoAction::CancelCountdown => {
                log::info!("Recording countdown cancelled");
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
