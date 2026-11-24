use crate::{App, Player, window};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

const SHOW: &str = "show";
const TOGGLE: &str = "toggle";
const NEXT: &str = "next";
const PREV: &str = "prev";
const QUIT: &str = "quit";

#[cfg(windows)]
mod instance {
    use windows_sys::Win32::{
        Foundation::{ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::Threading::{CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE},
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    pub struct Instance {
        _mutex: HANDLE,
        wake: HANDLE,
    }

    unsafe impl Send for Instance {}

    pub fn acquire() -> Option<Instance> {
        unsafe {
            let mutex = CreateMutexW(std::ptr::null(), 0, wide("Local\\RustifySingleInstance").as_ptr());
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let wake = OpenEventW(EVENT_MODIFY_STATE, 0, wide("Local\\RustifyActivate").as_ptr());
                if !wake.is_null() {
                    SetEvent(wake);
                }
                return None;
            }
            let wake = CreateEventW(std::ptr::null(), 0, 0, wide("Local\\RustifyActivate").as_ptr());
            Some(Instance { _mutex: mutex, wake })
        }
    }

    impl Instance {
        pub fn woken(&self) -> bool {
            unsafe { WaitForSingleObject(self.wake, 0) == WAIT_OBJECT_0 }
        }
    }
}

#[cfg(not(windows))]
mod instance {
    pub struct Instance;

    pub fn acquire() -> Option<Instance> {
        Some(Instance)
    }

    impl Instance {
        pub fn woken(&self) -> bool {
            false
        }
    }
}

pub use instance::{Instance, acquire};

pub fn show(app: &App) {
    let _ = app.show();
    app.window().set_minimized(false);
    app.window().with_winit_window(|w| w.focus_window());
}

pub fn hide(app: &App) {
    window::save(app);
    let _ = app.hide();
}

fn icon() -> Option<Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?.resize_exact(32, 32, image::imageops::FilterType::Lanczos3).to_rgba8();
    let (w, h) = image.dimensions();
    Icon::from_rgba(image.into_raw(), w, h).ok()
}

pub fn create() -> Option<TrayIcon> {
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &MenuItem::with_id(SHOW, "Show Rustify", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(TOGGLE, "Play / Pause", true, None),
        &MenuItem::with_id(NEXT, "Next", true, None),
        &MenuItem::with_id(PREV, "Previous", true, None),
        &PredefinedMenuItem::separator(),
        &MenuItem::with_id(QUIT, "Quit Rustify", true, None),
    ]);
    TrayIconBuilder::new()
        .with_tooltip("Rustify")
        .with_icon(icon()?)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .inspect_err(|e| tracing::warn!("tray: {e}"))
        .ok()
}

pub fn poll(app: &App, instance: &Instance) {
    if instance.woken() {
        show(app);
    }
    while let Ok(event) = TrayIconEvent::receiver().try_recv() {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
            show(app);
        }
    }
    while let Ok(event) = MenuEvent::receiver().try_recv() {
        let player = app.global::<Player>();
        match event.id().as_ref() {
            SHOW => show(app),
            TOGGLE => player.invoke_toggle(),
            NEXT => player.invoke_next(),
            PREV => player.invoke_prev(),
            QUIT => {
                window::save(app);
                let _ = slint::quit_event_loop();
            }
            _ => {}
        }
    }
}
