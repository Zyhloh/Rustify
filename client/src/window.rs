use crate::{App, Shell, config::Config};
use serde::{Deserialize, Serialize};
use slint::{ComponentHandle, PhysicalPosition, PhysicalSize, winit_030::WinitWindowAccessor};
use std::{cell::RefCell, path::PathBuf, time::Duration};

#[derive(Serialize, Deserialize)]
struct Geometry {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    maximized: bool,
}

struct Monitor {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
}

thread_local! {
    static NORMAL: RefCell<Option<Geometry>> = const { RefCell::new(None) };
}

fn path() -> PathBuf {
    Config::data_dir().join("window.json")
}

fn monitors(app: &App) -> Vec<Monitor> {
    app.window()
        .with_winit_window(|w| {
            w.available_monitors()
                .map(|m| Monitor {
                    x: m.position().x,
                    y: m.position().y,
                    width: m.size().width,
                    height: m.size().height,
                    scale: m.scale_factor(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn current_monitor(app: &App) -> Option<Monitor> {
    app.window().with_winit_window(|w| {
        w.current_monitor().or_else(|| w.primary_monitor()).map(|m| Monitor {
            x: m.position().x,
            y: m.position().y,
            width: m.size().width,
            height: m.size().height,
            scale: m.scale_factor(),
        })
    })?
}

pub fn restore_when_ready(weak: slint::Weak<App>) {
    slint::Timer::single_shot(std::time::Duration::from_millis(10), move || {
        let Some(app) = weak.upgrade() else { return };
        if app.window().has_winit_window() {
            restore(&app);
        } else {
            restore_when_ready(weak);
        }
    });
}

fn restore(app: &App) {
    style(app);
    watch_clicks(app);
    let window = app.window();

    let saved = std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Geometry>(&raw).ok())
        .filter(|g| {
            monitors(app).iter().any(|m| {
                g.x + 100 < m.x + m.width as i32
                    && g.x + g.width as i32 > m.x + 100
                    && g.y >= m.y - 20
                    && g.y < m.y + m.height as i32 - 100
            })
        });

    match saved {
        Some(g) => {
            window.set_size(PhysicalSize::new(g.width, g.height));
            window.set_position(PhysicalPosition::new(g.x, g.y));
            if g.maximized {
                window.set_maximized(true);
                app.global::<Shell>().set_maximized(true);
            }
        }
        None => {
            let Some(m) = current_monitor(app) else { return };
            let width = ((1600.0 * m.scale) as u32).min(m.width * 88 / 100);
            let height = ((960.0 * m.scale) as u32).min(m.height * 86 / 100);
            window.set_size(PhysicalSize::new(width, height));
            window.set_position(PhysicalPosition::new(
                m.x + (m.width as i32 - width as i32) / 2,
                m.y + (m.height as i32 - height as i32) / 2,
            ));
        }
    }
}

fn snapshot(app: &App) -> Geometry {
    let window = app.window();
    let (pos, size) = (window.position(), window.size());
    Geometry {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
        maximized: window.is_maximized(),
    }
}

fn apply(app: &App, g: &Geometry) {
    let window = app.window();
    window.set_size(PhysicalSize::new(g.width, g.height));
    window.set_position(PhysicalPosition::new(g.x, g.y));
    if g.maximized {
        window.set_maximized(true);
    }
    app.global::<Shell>().set_maximized(g.maximized);
}

fn set_on_top(app: &App, on_top: bool) {
    use slint::winit_030::winit::window::WindowLevel;
    app.window().with_winit_window(|w| {
        w.set_window_level(if on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal });
    });
}

pub fn enter_mini(app: &App) {
    let shell = app.global::<Shell>();
    if shell.get_mini() {
        return;
    }
    if shell.get_fullscreen() {
        toggle_fullscreen(app);
    }
    NORMAL.with(|n| *n.borrow_mut() = Some(snapshot(app)));
    if app.window().is_maximized() {
        app.window().set_maximized(false);
    }
    shell.set_maximized(false);
    shell.set_mini(true);
    set_on_top(app, true);
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::from_millis(40), move || {
        let Some(app) = weak.upgrade() else { return };
        let Some(m) = current_monitor(&app) else { return };
        let (width, height) = ((300.0 * m.scale) as u32, (340.0 * m.scale) as u32);
        let margin = (24.0 * m.scale) as i32;
        let bottom = (72.0 * m.scale) as i32;
        app.window().set_size(PhysicalSize::new(width, height));
        app.window().set_position(PhysicalPosition::new(
            m.x + m.width as i32 - width as i32 - margin,
            m.y + m.height as i32 - height as i32 - bottom,
        ));
    });
}

pub fn exit_mini(app: &App) {
    let shell = app.global::<Shell>();
    if !shell.get_mini() {
        return;
    }
    set_on_top(app, false);
    shell.set_mini(false);
    let previous = NORMAL.with(|n| n.borrow_mut().take());
    let weak = app.as_weak();
    slint::Timer::single_shot(Duration::from_millis(40), move || {
        let Some(app) = weak.upgrade() else { return };
        if let Some(g) = previous {
            apply(&app, &g);
        }
    });
}

pub fn toggle_fullscreen(app: &App) {
    let shell = app.global::<Shell>();
    let full = !shell.get_fullscreen();
    if full && shell.get_mini() {
        exit_mini(app);
    }
    app.window().set_fullscreen(full);
    shell.set_fullscreen(full);
}

pub fn save(app: &App) {
    if let Some(g) = NORMAL.with(|n| n.borrow_mut().take()) {
        write(&g);
        return;
    }
    if app.global::<Shell>().get_fullscreen() {
        return;
    }
    let window = app.window();
    let maximized = window.is_maximized();
    let previous = std::fs::read_to_string(path())
        .ok()
        .and_then(|raw| serde_json::from_str::<Geometry>(&raw).ok());

    let geometry = if maximized {
        match previous {
            Some(p) => Geometry { maximized: true, ..p },
            None => return,
        }
    } else {
        let (pos, size) = (window.position(), window.size());
        Geometry {
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            maximized: false,
        }
    };

    write(&geometry);
}

fn write(geometry: &Geometry) {
    if let Ok(raw) = serde_json::to_string(geometry) {
        let _ = std::fs::create_dir_all(Config::data_dir());
        let _ = std::fs::write(path(), raw);
    }
}

fn style(app: &App) {
    #[cfg(windows)]
    app.window().with_winit_window(|w| {
        use slint::winit_030::winit::platform::windows::{CornerPreference, WindowExtWindows};
        w.set_undecorated_shadow(true);
        w.set_corner_preference(CornerPreference::Round);
    });
}

fn watch_clicks(app: &App) {
    use slint::winit_030::{EventResult, winit::event::{ElementState, WindowEvent}};
    let weak = app.as_weak();
    let cursor = std::cell::Cell::new((0.0f64, 0.0f64));
    app.window().on_winit_window_event(move |window, event| {
        match event {
            WindowEvent::CursorMoved { position, .. } => cursor.set((position.x, position.y)),
            WindowEvent::HoveredFile(_) => {
                if let Some(app) = weak.upgrade() {
                    crate::plugins::hover_drop(&app, true);
                }
            }
            WindowEvent::HoveredFileCancelled => {
                if let Some(app) = weak.upgrade() {
                    crate::plugins::hover_drop(&app, false);
                }
            }
            WindowEvent::DroppedFile(path) => {
                if let Some(app) = weak.upgrade() {
                    crate::plugins::dropped(&app, path.clone());
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, .. } => {
                if let Some(app) = weak.upgrade() {
                    let store = app.global::<crate::Store>();
                    if store.get_search_focused() {
                        let scale = window.scale_factor() as f64;
                        let (x, y) = (cursor.get().0 / scale, cursor.get().1 / scale);
                        let left = app.get_search_left() as f64;
                        let right = left + app.get_search_width() as f64;
                        let inside = x >= left && x <= right && y <= app.get_search_bottom() as f64;
                        if !inside {
                            store.set_blur_search_request(store.get_blur_search_request() + 1);
                        }
                    }
                }
            }
            _ => {}
        }
        EventResult::Propagate
    });
}
