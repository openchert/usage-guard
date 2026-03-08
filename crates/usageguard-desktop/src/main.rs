#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chrono::Local;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewWindow,
};
use usageguard_core::{
    evaluate_alerts, load_config, provider_snapshots, save_config, set_provider_api_key,
    should_notify, AppConfig, UsageSnapshot,
};

struct AppState {
    cfg: Mutex<AppConfig>,
    last_notified: Mutex<HashMap<String, String>>,
}

const TRAY_TOGGLE_ID: &str = "tray.toggle";
const TRAY_QUIT_ID: &str = "tray.quit";
const CTX_REFRESH_ID: &str = "widget.refresh";
const CTX_ALWAYS_ON_TOP_ID: &str = "widget.always_on_top";
const CTX_HIDE_ID: &str = "widget.hide";
const CTX_QUIT_ID: &str = "widget.quit";
const REFRESH_EVENT: &str = "usageguard://refresh";

#[tauri::command]
fn get_snapshots(state: State<AppState>) -> Vec<UsageSnapshot> {
    let cfg = state.cfg.lock().unwrap().clone();
    let snapshots = provider_snapshots(&cfg);
    fire_notifications(&snapshots, &cfg, &mut state.last_notified.lock().unwrap());
    snapshots
}

#[tauri::command]
fn get_config(state: State<AppState>) -> AppConfig {
    state.cfg.lock().unwrap().clone()
}

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    set_provider_api_key(&provider, Some(&key)).map_err(|e| e.to_string())
}

#[tauri::command]
fn update_config(config: AppConfig, state: State<AppState>) -> Result<(), String> {
    save_config(&config).map_err(|e| e.to_string())?;
    *state.cfg.lock().unwrap() = config;
    Ok(())
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn show_context_menu(window: WebviewWindow, x: f64, y: f64) -> Result<(), String> {
    let menu = create_widget_menu(&window).map_err(|e| e.to_string())?;
    window
        .popup_menu_at(&menu, tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

/// Inline FFI bindings — no external crate needed, user32.dll is always present.
#[cfg(target_os = "windows")]
mod win32 {
    pub const SWP_NOACTIVATE: u32 = 0x0010;
    pub const SWP_NOZORDER: u32 = 0x0004;
    pub const SPI_GETWORKAREA: u32 = 0x0030;

    #[repr(C)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn SetWindowPos(
            hwnd: isize,
            hwnd_insert_after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;

        /// Retrieves system-wide parameters. Used here with SPI_GETWORKAREA to get
        /// the usable desktop area, which excludes the taskbar.
        pub fn SystemParametersInfoW(
            ui_action: u32,
            ui_param: u32,
            pv_param: *mut std::ffi::c_void,
            f_win_ini: u32,
        ) -> i32;
    }
}

/// Set window position and size in a single atomic OS call.
/// On Windows, SetWindowPos sets both in one call so DWM never composites an
/// intermediate frame — the previous two-call approach caused a one-frame flash.
/// Caller passes physical (device) pixel values.
#[tauri::command]
fn set_window_rect(window: tauri::WebviewWindow, x: i32, y: i32, w: i32, h: i32) {
    #[cfg(target_os = "windows")]
    {
        if let Ok(hwnd) = window.hwnd() {
            unsafe {
                win32::SetWindowPos(
                    hwnd.0 as isize,
                    0,
                    x,
                    y,
                    w,
                    h,
                    win32::SWP_NOACTIVATE | win32::SWP_NOZORDER,
                );
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        let _ = window.set_size(tauri::PhysicalSize::new(w.max(0) as u32, h.max(0) as u32));
    }
}

fn fire_notifications(
    snapshots: &[UsageSnapshot],
    cfg: &AppConfig,
    last_notified: &mut HashMap<String, String>,
) {
    for s in snapshots {
        if s.source == "demo" {
            continue;
        }
        let alerts = evaluate_alerts(s, cfg);
        if !should_notify(&alerts, Local::now(), cfg) {
            continue;
        }
        let sig = alerts
            .iter()
            .map(|a| format!("{}:{}", a.level, a.code))
            .collect::<Vec<_>>()
            .join(",");
        let changed = last_notified
            .get(&s.provider)
            .map(|x| x != &sig)
            .unwrap_or(true);
        if changed {
            last_notified.insert(s.provider.clone(), sig);
            emit_native_notification(
                "UsageGuard",
                &format!("{}: {}", s.account_label, alerts[0].message),
            );
        }
    }
}

#[cfg(target_os = "linux")]
fn emit_native_notification(title: &str, body: &str) {
    let _ = notify_rust::Notification::new()
        .summary(title)
        .body(body)
        .show();
}

#[cfg(target_os = "windows")]
fn emit_native_notification(title: &str, body: &str) {
    let _ =
        tauri_winrt_notification::Toast::new(tauri_winrt_notification::Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(body)
            .show();
}

#[cfg(target_os = "macos")]
fn emit_native_notification(title: &str, body: &str) {
    let _ = mac_notification_sys::send_notification(title, None, body, None);
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn emit_native_notification(_title: &str, _body: &str) {}

fn create_tray_icon() -> tauri::image::Image<'static> {
    const W: u32 = 16;
    const H: u32 = 16;
    let mut data = vec![0u8; (W * H * 4) as usize];
    let half = W as i32 / 2;
    let r2 = (half - 1).pow(2);
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let idx = ((y * W as i32 + x) * 4) as usize;
            let dx = x - half;
            let dy = y - half;
            if dx * dx + dy * dy <= r2 {
                data[idx] = 100;
                data[idx + 1] = 160;
                data[idx + 2] = 255;
                data[idx + 3] = 220;
            }
        }
    }
    // Leak into 'static so Image<'static> can borrow it
    let data: &'static [u8] = Box::leak(data.into_boxed_slice());
    tauri::image::Image::new(data, W, H)
}

fn create_widget_menu(window: &WebviewWindow) -> tauri::Result<Menu<tauri::Wry>> {
    let app = window.app_handle();
    let always_on_top = window.is_always_on_top().unwrap_or(true);
    let first_sep = PredefinedMenuItem::separator(app)?;
    let second_sep = PredefinedMenuItem::separator(app)?;

    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
            &first_sep,
            &CheckMenuItem::with_id(
                app,
                CTX_ALWAYS_ON_TOP_ID,
                "Always on Top",
                true,
                always_on_top,
                None::<&str>,
            )?,
            &MenuItem::with_id(app, CTX_HIDE_ID, "Hide to Tray", true, None::<&str>)?,
            &second_sep,
            &MenuItem::with_id(app, CTX_QUIT_ID, "Quit", true, None::<&str>)?,
        ],
    )
}

fn toggle_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        TRAY_TOGGLE_ID => toggle_window(app),
        TRAY_QUIT_ID | CTX_QUIT_ID => app.exit(0),
        CTX_REFRESH_ID => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.emit(REFRESH_EVENT, ());
            }
        }
        CTX_ALWAYS_ON_TOP_ID => {
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(current) = win.is_always_on_top() {
                    let _ = win.set_always_on_top(!current);
                }
            }
        }
        CTX_HIDE_ID => {
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }
        }
        _ => {}
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let cfg = load_config().unwrap_or_default();
            app.manage(AppState {
                cfg: Mutex::new(cfg),
                last_notified: Mutex::new(HashMap::new()),
            });

            // Position widget at bottom-right of the work area (excludes taskbar on Windows)
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(Some(monitor)) = win.current_monitor() {
                    let scale = monitor.scale_factor();

                    // On Windows, use SystemParametersInfo(SPI_GETWORKAREA) so the widget
                    // lands above the taskbar instead of behind it.
                    #[cfg(target_os = "windows")]
                    let (area_w, area_h) = {
                        let mut rect = win32::Rect {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        };
                        unsafe {
                            win32::SystemParametersInfoW(
                                win32::SPI_GETWORKAREA,
                                0,
                                &mut rect as *mut _ as *mut _,
                                0,
                            );
                        }
                        (rect.right as f64 / scale, rect.bottom as f64 / scale)
                    };

                    #[cfg(not(target_os = "windows"))]
                    let (area_w, area_h) = {
                        let size = monitor.size();
                        (size.width as f64 / scale, size.height as f64 / scale)
                    };

                    let widget_w = 244.0;
                    let widget_h = 100.0;
                    let margin_right = 20.0;
                    let margin_bottom = 12.0;
                    let _ = win.set_position(tauri::LogicalPosition::new(
                        area_w - widget_w - margin_right,
                        area_h - widget_h - margin_bottom,
                    ));
                }
            }

            app.on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()));

            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, TRAY_TOGGLE_ID, "Show / Hide", true, None::<&str>)?,
                    &sep,
                    &MenuItem::with_id(app, TRAY_QUIT_ID, "Quit UsageGuard", true, None::<&str>)?,
                ],
            )?;

            TrayIconBuilder::new()
                .icon(create_tray_icon())
                .tooltip("UsageGuard")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshots,
            get_config,
            save_api_key,
            update_config,
            quit,
            show_context_menu,
            set_window_rect,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
