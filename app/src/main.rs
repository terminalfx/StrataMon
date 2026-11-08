//! The window around the interface: input goes in, pixels come out, and worker threads do
//! anything slow. Everything that decides what happens lives in the core and ui crates.

#![windows_subsystem = "windows"]

mod monitor;
mod platform;
mod transport;
mod updater;

use monitor::Monitor;
use platform::Notifier;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};
use stratamon_core::client::PollResult;
use stratamon_core::config::{self, AppConfig};
use stratamon_core::format;
use stratamon_core::update::{self, UpdateStatus};
use stratamon_ui::text::Text;
use stratamon_ui::{Command, Cursor, DEFAULT_HEIGHT, DEFAULT_WIDTH, Input, Key, Ui};
use tiny_skia::Pixmap;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, Key as WinitKey, ModifiersState, NamedKey, PhysicalKey};
use winit::window::{CursorIcon, Theme, Window, WindowId};

const MIN_WIDTH: f64 = 420.0;
const MIN_HEIGHT: f64 = 480.0;

enum AppEvent {
    Poll { generation: u64, index: usize, result: Box<PollResult> },
    TestResult { form_id: String, text: String },
    Update(UpdateStatus),

    /// A verified update has been swapped in and started, so this copy can close.
    Restart,
}

struct Screen {
    window: Rc<Window>,
    surface: softbuffer::Surface<Rc<Window>, Rc<Window>>,
    frame: Pixmap,
    notifier: Notifier,
}

struct App {
    proxy: EventLoopProxy<AppEvent>,
    config_path: PathBuf,
    ui: Ui,
    screen: Option<Screen>,
    monitor: Option<Monitor>,
    clipboard: Option<arboard::Clipboard>,
    modifiers: ModifiersState,
}

fn unix_now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

impl App {
    fn start_monitor(&mut self) {
        self.monitor = None;
        let (proxy, generation) = (self.proxy.clone(), self.ui.dashboard.generation);
        let report = move |index, result| proxy.send_event(AppEvent::Poll { generation, index, result: Box::new(result) }).is_ok();
        self.monitor = Some(Monitor::start(&self.ui.dashboard.config, report));
    }

    fn present(&mut self) {
        let Some(screen) = &mut self.screen else {
            return;
        };

        let (width, height) = (screen.frame.width(), screen.frame.height());
        let (Some(surface_width), Some(surface_height)) = (NonZeroU32::new(width), NonZeroU32::new(height)) else {
            return;
        };

        self.ui.draw(&mut screen.frame);

        if screen.surface.resize(surface_width, surface_height).is_err() {
            return;
        }

        let Ok(mut buffer) = screen.surface.buffer_mut() else {
            return;
        };

        for (target, pixel) in buffer.iter_mut().zip(screen.frame.data().as_chunks::<4>().0) {
            *target = u32::from(pixel[0]) << 16 | u32::from(pixel[1]) << 8 | u32::from(pixel[2]);
        }

        let _ = buffer.present();
    }

    fn after_change(&mut self) {
        if let Some(screen) = &self.screen {
            screen.window.set_cursor(match self.ui.cursor() {
                Cursor::Normal => CursorIcon::Default,
                Cursor::Hand => CursorIcon::Pointer,
                Cursor::Text => CursorIcon::Text,
                Cursor::Resize => CursorIcon::NsResize,
            });

            if self.ui.needs_redraw() {
                screen.window.request_redraw();
            }
        }
    }

    fn input(&mut self, input: Input) {
        let commands = self.ui.handle(input);
        for command in commands {
            self.run(command);
        }
        self.after_change();
    }

    fn run(&mut self, command: Command) {
        match command {
            Command::SaveSettings(config) => self.save(config),
            Command::TestConnection { form_id, server } => {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let text = monitor::test_connection(&server);
                    let _ = proxy.send_event(AppEvent::TestResult { form_id, text });
                });
            }
            Command::SendTestNotification => {
                if let Some(screen) = &mut self.screen {
                    screen.notifier.show("This is a test notification from StrataMon.", false);
                }
            }
            Command::CheckForUpdates => self.in_background(updater::check),
            Command::InstallUpdate { tag } => {
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let _ = proxy.send_event(match updater::install(&tag) {
                        Ok(()) => AppEvent::Restart,
                        Err(message) => AppEvent::Update(UpdateStatus::InstallFailed { message, link: update::release_link(&tag) }),
                    });
                });
            }
            Command::OpenLink(url) => platform::open_link(&url),
            Command::CopyText(text) => {
                if let Some(clipboard) = self.clipboard() {
                    let _ = clipboard.set_text(text);
                }
            }
            Command::RequestPaste => {
                if let Some(text) = self.clipboard().and_then(|clipboard| clipboard.get_text().ok()) {
                    self.input(Input::Paste(text));
                }
            }
        }
    }

    fn in_background(&self, work: impl FnOnce() -> UpdateStatus + Send + 'static) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(AppEvent::Update(work()));
        });
    }

    // Opened on first use, so nothing touches the clipboard until someone copies or pastes.
    fn clipboard(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.clipboard.is_none() {
            self.clipboard = arboard::Clipboard::new().ok();
        }
        self.clipboard.as_mut()
    }

    fn save(&mut self, config: AppConfig) {
        self.ui.dashboard.config_notice = config::save(&self.config_path, &config)
            .err()
            // Still use the new settings for this session, and say they won't last.
            .map(|error| format!("Your settings couldn't be saved, so they'll be lost when StrataMon closes. {error}"));

        self.ui.dashboard.watch(config);
        self.start_monitor();
    }

    fn key(&mut self, key: &WinitKey, physical: PhysicalKey, text: Option<&str>) {
        let shift = self.modifiers.shift_key();
        let control = self.modifiers.control_key() && !self.modifiers.alt_key();

        let named = match key {
            WinitKey::Named(NamedKey::ArrowLeft) => Some(Key::Left),
            WinitKey::Named(NamedKey::ArrowRight) => Some(Key::Right),
            WinitKey::Named(NamedKey::Home) => Some(Key::Home),
            WinitKey::Named(NamedKey::End) => Some(Key::End),
            WinitKey::Named(NamedKey::Backspace) => Some(Key::Backspace),
            WinitKey::Named(NamedKey::Delete) => Some(Key::Delete),
            WinitKey::Named(NamedKey::Tab) => Some(Key::Tab),
            WinitKey::Named(NamedKey::Enter) => Some(Key::Enter),
            WinitKey::Named(NamedKey::Escape) => Some(Key::Escape),
            WinitKey::Character(letter) if control => letter.chars().next().map(|c| Key::Control(c.to_ascii_lowercase())),
            // With Ctrl held, a layout or IME can swallow the character entirely and leave no
            // text to match. The physical key is layout-independent, so the four shortcuts
            // the settings fields offer still work everywhere.
            WinitKey::Unidentified(_) if control => match physical {
                PhysicalKey::Code(KeyCode::KeyA) => Some(Key::Control('a')),
                PhysicalKey::Code(KeyCode::KeyC) => Some(Key::Control('c')),
                PhysicalKey::Code(KeyCode::KeyX) => Some(Key::Control('x')),
                PhysicalKey::Code(KeyCode::KeyV) => Some(Key::Control('v')),
                _ => None,
            },
            _ => None,
        };

        if let Some(key) = named {
            self.input(Input::Key { key, shift });
        } else if !control && matches!(key, WinitKey::Character(_) | WinitKey::Named(NamedKey::Space))
            && let Some(typed) = text {
                self.input(Input::Text(typed.to_string()));
            }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.screen.is_some() {
            return;
        }

        let attributes = Window::default_attributes()
            .with_title("StrataMon")
            .with_window_icon(platform::window_icon())
            .with_inner_size(LogicalSize::new(f64::from(DEFAULT_WIDTH), f64::from(DEFAULT_HEIGHT)))
            .with_min_inner_size(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT));

        let Ok(window) = event_loop.create_window(attributes).map(Rc::new) else {
            eprintln!("StrataMon couldn't open a window.");
            event_loop.exit();
            return;
        };

        let Ok(surface) = softbuffer::Context::new(window.clone()).and_then(|context| softbuffer::Surface::new(&context, window.clone())) else {
            eprintln!("StrataMon couldn't draw to its window.");
            event_loop.exit();
            return;
        };

        self.ui.set_dark(window.theme() == Some(Theme::Dark));

        let size = window.inner_size();
        self.ui.set_viewport(size.width, size.height, window.scale_factor() as f32);
        let Some(frame) = Pixmap::new(size.width.max(1), size.height.max(1)) else {
            event_loop.exit();
            return;
        };

        let notifier = Notifier::new(&window);
        window.request_redraw();
        self.screen = Some(Screen { window, surface, frame, notifier });
        self.start_monitor();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        match event {
            AppEvent::Poll { generation, index, result } => {
                format::set_utc_offset(platform::utc_offset_seconds());
                let notifications = self.ui.apply_poll(generation, index, *result, unix_now());
                if let Some(screen) = &mut self.screen {
                    for notification in notifications {
                        screen.notifier.show(&notification.message, notification.critical);
                    }
                }
            }
            AppEvent::TestResult { form_id, text } => self.ui.set_test_result(&form_id, text),
            AppEvent::Update(status) => self.ui.set_update_status(status),
            AppEvent::Restart => event_loop.exit(),
        }

        self.after_change();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::RedrawRequested => self.present(),
            // Minimizing on Windows reports a size of nothing. There's nothing to draw then, and
            // laying the window out for it would squeeze everything that has a minimum size.
            WindowEvent::Resized(size) if size.width == 0 || size.height == 0 => {}
            WindowEvent::Resized(size) => {
                if let Some(screen) = &mut self.screen {
                    if let Some(frame) = Pixmap::new(size.width.max(1), size.height.max(1)) {
                        screen.frame = frame;
                    }
                    self.ui.set_viewport(size.width, size.height, screen.window.scale_factor() as f32);
                }
                self.after_change();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(screen) = &self.screen {
                    let size = screen.window.inner_size();
                    self.ui.set_viewport(size.width, size.height, scale_factor as f32);
                }
                self.after_change();
            }
            WindowEvent::ThemeChanged(theme) => {
                self.ui.set_dark(theme == Theme::Dark);
                self.after_change();
            }
            WindowEvent::CursorMoved { position, .. } => self.input(Input::MouseMove { x: position.x as f32, y: position.y as f32 }),
            WindowEvent::CursorLeft { .. } => self.input(Input::MouseLeft),
            WindowEvent::MouseInput { state, button: MouseButton::Left, .. } => {
                self.input(if state == ElementState::Pressed { Input::MouseDown } else { Input::MouseUp });
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, lines) => lines,
                    MouseScrollDelta::PixelDelta(pixels) => (pixels.y / f64::from(stratamon_ui::WHEEL_LINE)) as f32,
                };
                self.input(Input::Wheel { lines });
            }
            WindowEvent::ModifiersChanged(modifiers) => self.modifiers = modifiers.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                self.key(&event.logical_key, event.physical_key, event.text.as_deref());
            }
            _ => {}
        }
    }
}

/// The settings file to use, and a message if saved settings couldn't be brought over to it.
fn config_path() -> (PathBuf, Option<String>) {
    let arguments: Vec<String> = std::env::args().collect();
    let chosen = arguments.iter().position(|argument| argument == "--config").and_then(|index| arguments.get(index + 1));
    if let Some(path) = chosen {
        return (PathBuf::from(path), None);
    }

    let (old, path) = (config::old_default_path(), config::default_path());
    let notice = config::adopt_old_settings(&old, &path)
        .err()
        .map(|error| format!("Your saved settings couldn't be copied from {}, so StrataMon started fresh. They're still there. {error}", old.display()));
    (path, notice)
}

fn main() {
    updater::clean_up_after_update();

    let (config_path, adopt_notice) = config_path();
    let (config, moved_to) = config::load(&config_path);

    // Plain settings an older build left behind, or one copied in by hand, get sealed now:
    // the keys stop being plain text without waiting for the user to change a setting.
    let seal_notice = if config::needs_sealing(&config_path) {
        config::save(&config_path, &config)
            .err()
            .map(|error| format!("Your settings couldn't be sealed to this Windows account, so they're still plain text on disk. {error}"))
    } else {
        None
    };

    let text = match Text::from_system() {
        Ok(text) => text,
        Err(problem) => {
            eprintln!("{problem}");
            std::process::exit(1);
        }
    };

    let Ok(event_loop) = EventLoop::<AppEvent>::with_user_event().build() else {
        eprintln!("StrataMon couldn't start its window system.");
        std::process::exit(1);
    };

    let mut ui = Ui::new(config, text, false);
    ui.dashboard.config_notice = adopt_notice
        .or(seal_notice)
        .or_else(|| moved_to.map(|path| format!("Your settings file couldn't be read, so it was kept at {} and StrataMon started fresh.", path.display())));

    let mut app = App {
        proxy: event_loop.create_proxy(),
        config_path,
        ui,
        screen: None,
        monitor: None,
        clipboard: None,
        modifiers: ModifiersState::default(),
    };

    format::set_utc_offset(platform::utc_offset_seconds());
    let _ = event_loop.run_app(&mut app);
}
