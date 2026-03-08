#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon_art;

use chrono::Local;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use usageguard_core::{
    evaluate_alerts, has_provider_account_api_key, load_config, provider_catalog,
    provider_snapshots, save_config, set_provider_account_api_key, set_provider_api_key,
    should_notify, AppConfig, ProviderAccount, ProviderCatalogEntry, UsageSnapshot,
};

struct AppState {
    cfg: Mutex<AppConfig>,
    last_notified: Mutex<HashMap<String, String>>,
}

const TRAY_TOGGLE_ID: &str = "tray.toggle";
const TRAY_PROVIDERS_ID: &str = "tray.providers";
const TRAY_QUIT_ID: &str = "tray.quit";
const CTX_REFRESH_ID: &str = "widget.refresh";
const CTX_PROVIDERS_ID: &str = "widget.providers";
const CTX_ALWAYS_ON_TOP_ID: &str = "widget.always_on_top";
const CTX_HIDE_ID: &str = "widget.hide";
const CTX_QUIT_ID: &str = "widget.quit";
const REFRESH_EVENT: &str = "usageguard://refresh";
const SETTINGS_LABEL: &str = "settings";

#[derive(Debug, Clone, Serialize)]
struct ProviderAccountView {
    id: String,
    provider: String,
    provider_label: String,
    label: String,
    endpoint: Option<String>,
    default_endpoint: Option<String>,
    has_api_key: bool,
    endpoint_required: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ProviderSettingsPayload {
    providers: Vec<ProviderCatalogEntry>,
    accounts: Vec<ProviderAccountView>,
}

#[derive(Debug, Deserialize)]
struct ProviderAccountInput {
    id: Option<String>,
    provider: String,
    label: String,
    endpoint: Option<String>,
    api_key: Option<String>,
}

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

fn normalize_optional(value: Option<String>) -> Option<String> {
    value.and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_required(value: &str, field: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{field} is required"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn make_account_id(provider: &str, label: &str) -> String {
    let slug: String = label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "account".to_string()
    } else {
        slug
    };
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("acct_{provider}_{slug}_{ts}")
}

fn provider_settings_payload(cfg: &AppConfig) -> ProviderSettingsPayload {
    let providers = provider_catalog();
    let provider_map: HashMap<&str, &ProviderCatalogEntry> = providers
        .iter()
        .map(|provider| (provider.id.as_str(), provider))
        .collect();

    let mut accounts = cfg
        .provider_accounts
        .iter()
        .filter_map(|account| {
            let meta = provider_map.get(account.provider.as_str())?;
            Some(ProviderAccountView {
                id: account.id.clone(),
                provider: account.provider.clone(),
                provider_label: meta.label.clone(),
                label: account.label.clone(),
                endpoint: account.endpoint.clone(),
                default_endpoint: meta.default_endpoint.clone(),
                has_api_key: has_provider_account_api_key(&account.id),
                endpoint_required: meta.endpoint_required,
            })
        })
        .collect::<Vec<_>>();

    accounts.sort_by(|a, b| {
        a.provider_label
            .cmp(&b.provider_label)
            .then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    });

    ProviderSettingsPayload {
        providers,
        accounts,
    }
}

fn emit_widget_refresh(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit(REFRESH_EVENT, ());
    }
}

fn open_provider_settings_impl(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    const SETTINGS_W: f64 = 360.0;
    const SETTINGS_H: f64 = 480.0;
    const GAP: f64 = 8.0;

    // Position the settings window just above the main widget, right-aligned.
    let position = app.get_webview_window("main").and_then(|main_win| {
        let scale = main_win.scale_factor().ok()?;
        let phys_pos = main_win.outer_position().ok()?;
        let phys_size = main_win.inner_size().ok()?;
        let widget_x = phys_pos.x as f64 / scale;
        let widget_y = phys_pos.y as f64 / scale;
        let widget_w = phys_size.width as f64 / scale;
        let x = (widget_x + widget_w - SETTINGS_W).max(0.0);
        let y = (widget_y - SETTINGS_H - GAP).max(0.0);
        Some((x, y))
    });

    let builder = WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("index.html?view=settings".into()),
    )
    .title("UsageGuard Providers")
    .inner_size(SETTINGS_W, SETTINGS_H)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .always_on_top(false)
    .skip_taskbar(false)
    .shadow(false)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .focused(true);

    let builder = match position {
        Some((x, y)) => builder.position(x, y),
        None => builder.center(),
    };

    builder.build().map_err(|e| e.to_string())?;

    Ok(())
}

fn spawn_open_provider_settings(app: AppHandle) {
    std::thread::spawn(move || {
        if let Err(error) = open_provider_settings_impl(&app) {
            eprintln!("failed to open provider settings: {error}");
        }
    });
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
fn open_provider_settings(app: AppHandle) {
    spawn_open_provider_settings(app);
}

#[tauri::command]
fn get_provider_settings(state: State<AppState>) -> ProviderSettingsPayload {
    provider_settings_payload(&state.cfg.lock().unwrap().clone())
}

#[tauri::command]
fn save_provider_account(
    input: ProviderAccountInput,
    state: State<AppState>,
    app: AppHandle,
) -> Result<ProviderSettingsPayload, String> {
    let provider = normalize_required(&input.provider, "Provider")?;
    let label = normalize_required(&input.label, "Name")?;
    let endpoint = normalize_optional(input.endpoint);
    let api_key = normalize_optional(input.api_key);

    let catalog = provider_catalog();
    let provider_meta = catalog
        .iter()
        .find(|entry| entry.id == provider)
        .ok_or_else(|| "Unknown provider selected".to_string())?;

    if provider_meta.endpoint_required && endpoint.is_none() {
        return Err(format!("{} requires an endpoint URL", provider_meta.label));
    }

    let mut cfg = state.cfg.lock().unwrap().clone();
    let existing_index = input.id.as_ref().and_then(|id| {
        cfg.provider_accounts
            .iter()
            .position(|account| account.id == *id)
    });

    if cfg
        .provider_accounts
        .iter()
        .enumerate()
        .any(|(index, account)| {
            Some(index) != existing_index
                && account.provider == provider
                && account.label.eq_ignore_ascii_case(&label)
        })
    {
        return Err(format!(
            "A {} account named '{}' already exists",
            provider_meta.label, label
        ));
    }

    let account_id = existing_index
        .and_then(|index| {
            cfg.provider_accounts
                .get(index)
                .map(|account| account.id.clone())
        })
        .unwrap_or_else(|| make_account_id(&provider, &label));

    if api_key.is_none() && !has_provider_account_api_key(&account_id) {
        return Err("API key is required".to_string());
    }

    if let Some(ref key) = api_key {
        set_provider_account_api_key(&account_id, Some(key)).map_err(|e| e.to_string())?;
    }

    let account = ProviderAccount {
        id: account_id,
        provider,
        label,
        endpoint,
    };

    if let Some(index) = existing_index {
        cfg.provider_accounts[index] = account;
    } else {
        cfg.provider_accounts.push(account);
    }

    save_config(&cfg).map_err(|e| e.to_string())?;
    *state.cfg.lock().unwrap() = cfg.clone();
    emit_widget_refresh(&app);
    Ok(provider_settings_payload(&cfg))
}

#[tauri::command]
fn delete_provider_account(
    id: String,
    state: State<AppState>,
    app: AppHandle,
) -> Result<ProviderSettingsPayload, String> {
    let mut cfg = state.cfg.lock().unwrap().clone();
    let before = cfg.provider_accounts.len();
    cfg.provider_accounts.retain(|account| account.id != id);
    if cfg.provider_accounts.len() == before {
        return Err("Provider account not found".to_string());
    }

    save_config(&cfg).map_err(|e| e.to_string())?;
    let _ = set_provider_account_api_key(&id, None);
    *state.cfg.lock().unwrap() = cfg.clone();
    emit_widget_refresh(&app);
    Ok(provider_settings_payload(&cfg))
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
        let notification_key = format!("{}::{}", s.provider, s.account_label);
        let changed = last_notified
            .get(&notification_key)
            .map(|x| x != &sig)
            .unwrap_or(true);
        if changed {
            last_notified.insert(notification_key, sig);
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
    static PIXELS: OnceLock<Box<[u8]>> = OnceLock::new();
    let size = icon_art::TRAY_ICON_SIZE;
    let data = PIXELS.get_or_init(|| icon_art::icon_rgba_pixels(size).into_boxed_slice());
    tauri::image::Image::new(data, size, size)
}

fn create_widget_menu(window: &WebviewWindow) -> tauri::Result<Menu<tauri::Wry>> {
    let app = window.app_handle();
    let always_on_top = window.is_always_on_top().unwrap_or(true);
    let first_sep = PredefinedMenuItem::separator(app)?;
    let second_sep = PredefinedMenuItem::separator(app)?;
    let third_sep = PredefinedMenuItem::separator(app)?;

    Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
            &first_sep,
            &MenuItem::with_id(
                app,
                CTX_PROVIDERS_ID,
                "Manage Providers...",
                true,
                None::<&str>,
            )?,
            &second_sep,
            &CheckMenuItem::with_id(
                app,
                CTX_ALWAYS_ON_TOP_ID,
                "Always on Top",
                true,
                always_on_top,
                None::<&str>,
            )?,
            &MenuItem::with_id(app, CTX_HIDE_ID, "Hide to Tray", true, None::<&str>)?,
            &third_sep,
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
        TRAY_PROVIDERS_ID | CTX_PROVIDERS_ID => spawn_open_provider_settings(app.clone()),
        TRAY_QUIT_ID | CTX_QUIT_ID => app.exit(0),
        CTX_REFRESH_ID => {
            emit_widget_refresh(app);
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
            let providers_sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, TRAY_TOGGLE_ID, "Show / Hide", true, None::<&str>)?,
                    &sep,
                    &MenuItem::with_id(
                        app,
                        TRAY_PROVIDERS_ID,
                        "Manage Providers...",
                        true,
                        None::<&str>,
                    )?,
                    &providers_sep,
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
            get_provider_settings,
            open_provider_settings,
            save_provider_account,
            delete_provider_account,
            save_api_key,
            update_config,
            quit,
            show_context_menu,
            set_window_rect,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
