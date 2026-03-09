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
    provider_snapshots, save_config, set_provider_account_api_key, should_notify, AppConfig,
    ProviderAccount, ProviderCatalogEntry, UsageSnapshot,
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
    has_api_key: bool,
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

fn require_window_label(
    window: &WebviewWindow,
    expected_label: &str,
    command_name: &str,
) -> Result<(), String> {
    if window.label() == expected_label {
        Ok(())
    } else {
        Err(format!(
            "{command_name} is only available from the {expected_label} window"
        ))
    }
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
                has_api_key: has_provider_account_api_key(&account.id),
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
        let _ = win.set_always_on_top(true);
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }

    const SETTINGS_W: f64 = 360.0;
    const SETTINGS_H: f64 = 540.0;
    const GAP: f64 = 8.0;

    // Position the settings window to the left of the main widget, bottom-aligned.
    // No clamping so it follows the widget to whichever monitor it lives on.
    let position = app.get_webview_window("main").and_then(|main_win| {
        let scale = main_win.scale_factor().ok()?;
        let phys_pos = main_win.outer_position().ok()?;
        let phys_size = main_win.inner_size().ok()?;
        let widget_x = phys_pos.x as f64 / scale;
        let widget_y = phys_pos.y as f64 / scale;
        let widget_h = phys_size.height as f64 / scale;
        // Left of the widget, bottom-aligned with the widget background edge.
        // +8 compensates for the settings shell padding so the visible panel
        // bottom lines up with the widget window bottom.
        let x = widget_x - SETTINGS_W - GAP;
        let y = widget_y + widget_h - SETTINGS_H + 8.0;
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
    .always_on_top(true)
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
fn update_config(
    window: WebviewWindow,
    config: AppConfig,
    state: State<AppState>,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "update_config")?;
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
    window: WebviewWindow,
    input: ProviderAccountInput,
    state: State<AppState>,
    app: AppHandle,
) -> Result<ProviderSettingsPayload, String> {
    require_window_label(&window, SETTINGS_LABEL, "save_provider_account")?;
    let provider = normalize_required(&input.provider, "Provider")?;
    let label = normalize_required(&input.label, "Name")?;
    let api_key = normalize_optional(input.api_key);

    let catalog = provider_catalog();
    let provider_meta = catalog
        .iter()
        .find(|entry| entry.id == provider)
        .ok_or_else(|| "Unknown provider selected".to_string())?;

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
        endpoint: None,
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
    window: WebviewWindow,
    id: String,
    state: State<AppState>,
    app: AppHandle,
) -> Result<ProviderSettingsPayload, String> {
    require_window_label(&window, SETTINGS_LABEL, "delete_provider_account")?;
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

/// Saves the current widget position to config, then exits.
/// Called from every quit path so the position is always persisted.
fn save_position_and_exit(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if let (Ok(pos), Ok(scale)) = (win.outer_position(), win.scale_factor()) {
            let state = app.state::<AppState>();
            let mut cfg = state.cfg.lock().unwrap().clone();
            cfg.widget_position = Some([pos.x as f64 / scale, pos.y as f64 / scale]);
            let _ = save_config(&cfg);
            *state.cfg.lock().unwrap() = cfg;
        }
    }
    app.exit(0);
}

#[tauri::command]
fn quit(app: AppHandle) {
    save_position_and_exit(&app);
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
        TRAY_QUIT_ID | CTX_QUIT_ID => save_position_and_exit(app),
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

// --- OAuth PKCE helpers ---

#[derive(Clone, Copy)]
enum OAuthTokenEncoding {
    Form,
    Json,
}

#[derive(Clone, Copy)]
struct OAuthProviderConfig {
    client_id: &'static str,
    authorize_url: &'static str,
    token_url: &'static str,
    callback_host: &'static str,
    callback_port: u16,
    callback_path: &'static str,
    scope: &'static str,
    token_encoding: OAuthTokenEncoding,
    include_state_in_token_request: bool,
    auth_extra_params: &'static [(&'static str, &'static str)],
}

const OPENAI_OAUTH_AUTH_EXTRA_PARAMS: [(&str, &str); 2] = [
    ("id_token_add_organizations", "true"),
    ("codex_cli_simplified_flow", "true"),
];

const ANTHROPIC_OAUTH_AUTH_EXTRA_PARAMS: [(&str, &str); 0] = [];

const OPENAI_OAUTH_PROVIDER: OAuthProviderConfig = OAuthProviderConfig {
    client_id: "app_EMoamEEZ73f0CkXaXp7hrann",
    authorize_url: "https://auth.openai.com/oauth/authorize",
    token_url: "https://auth.openai.com/oauth/token",
    callback_host: "localhost",
    callback_port: 1455,
    callback_path: "/auth/callback",
    scope: "openid profile email offline_access",
    token_encoding: OAuthTokenEncoding::Form,
    include_state_in_token_request: false,
    auth_extra_params: &OPENAI_OAUTH_AUTH_EXTRA_PARAMS,
};

const ANTHROPIC_OAUTH_PROVIDER: OAuthProviderConfig = OAuthProviderConfig {
    client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
    authorize_url: "https://claude.ai/oauth/authorize",
    token_url: "https://platform.claude.com/v1/oauth/token",
    callback_host: "localhost",
    callback_port: 45454,
    callback_path: "/callback",
    scope: "user:inference user:mcp_servers user:profile user:sessions:claude_code",
    token_encoding: OAuthTokenEncoding::Json,
    include_state_in_token_request: true,
    auth_extra_params: &ANTHROPIC_OAUTH_AUTH_EXTRA_PARAMS,
};

fn pkce_verifier() -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

fn oauth_redirect_uri(provider: OAuthProviderConfig) -> String {
    format!(
        "http://{}:{}{}",
        provider.callback_host, provider.callback_port, provider.callback_path
    )
}

fn bind_oauth_listener(
    provider: OAuthProviderConfig,
) -> Result<(std::net::TcpListener, String), String> {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", provider.callback_port)).map_err(|e| {
            format!(
                "Unable to bind loopback callback on port {}: {e}",
                provider.callback_port
            )
        })?;
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("Unable to configure loopback callback: {e}"))?;
    Ok((listener, oauth_redirect_uri(provider)))
}

fn oauth_callback_body(message: &str) -> String {
    format!("<html><body><h2>{message}</h2></body></html>")
}

fn parse_callback_code(
    target: &str,
    callback_path: &str,
    expected_state: &str,
) -> Result<String, String> {
    let path = target.split('?').next().unwrap_or_default();
    if path != callback_path {
        return Err("Unexpected callback path".to_string());
    }

    let query = target.split('?').nth(1).unwrap_or_default();
    let params: HashMap<String, String> = url::form_urlencoded::parse(query.as_bytes())
        .into_owned()
        .collect();

    if let Some(error) = params.get("error") {
        return Err(format!("OAuth callback returned error: {error}"));
    }

    let state = params
        .get("state")
        .ok_or_else(|| "Missing state in callback URL".to_string())?;
    if state != expected_state {
        return Err("OAuth state mismatch".to_string());
    }

    params
        .get("code")
        .cloned()
        .ok_or_else(|| "Missing authorization code in callback URL".to_string())
}

/// Blocks until the browser hits the local callback, or 5 minutes elapse.
fn wait_for_callback(
    listener: std::net::TcpListener,
    callback_path: String,
    expected_state: String,
) -> Result<String, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(300);

    loop {
        if Instant::now() >= deadline {
            return Err("Timed out waiting for browser sign-in (5 minutes)".into());
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let mut reader = BufReader::new(&stream);

                let mut first_line = String::new();
                let _ = reader.read_line(&mut first_line);

                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line == "\r\n" => break,
                        _ => {}
                    }
                }

                let target = first_line
                    .split_whitespace()
                    .nth(1)
                    .ok_or_else(|| "Malformed callback request".to_string())?;
                let code = parse_callback_code(target, &callback_path, &expected_state);
                let body = oauth_callback_body(match &code {
                    Ok(_) => "Connected to UsageGuard. You can close this tab.",
                    Err(_) => "Sign-in failed. You can close this tab and try again.",
                });
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                return code;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn oauth_response_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|entry| entry.as_str())
            .map(str::to_string)
    })
}

fn oauth_tokens_from_response(
    response: &serde_json::Value,
) -> Result<(String, String, chrono::DateTime<chrono::Utc>), String> {
    let access = response["access_token"]
        .as_str()
        .ok_or("No access_token in response")?
        .to_string();
    let refresh = response["refresh_token"].as_str().unwrap_or("").to_string();
    let expires_in = response["expires_in"].as_i64().unwrap_or(45 * 60).max(61);
    let expires_at = chrono::Utc::now() + chrono::Duration::seconds(expires_in - 60);
    Ok((access, refresh, expires_at))
}

/// Exchanges the auth code for access + refresh tokens.
fn exchange_code(
    provider: OAuthProviderConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    state: &str,
) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let request = client.post(provider.token_url);
    let response = match provider.token_encoding {
        OAuthTokenEncoding::Form => {
            let params = vec![
                ("grant_type", "authorization_code".to_string()),
                ("code", code.to_string()),
                ("code_verifier", verifier.to_string()),
                ("redirect_uri", redirect_uri.to_string()),
                ("client_id", provider.client_id.to_string()),
            ];
            request.form(&params)
        }
        OAuthTokenEncoding::Json => {
            let mut body = serde_json::json!({
                "grant_type": "authorization_code",
                "code": code,
                "code_verifier": verifier,
                "redirect_uri": redirect_uri,
                "client_id": provider.client_id,
            });
            if provider.include_state_in_token_request {
                body["state"] = serde_json::Value::String(state.to_string());
            }
            request
                .header("Content-Type", "application/json")
                .json(&body)
        }
    };

    response
        .send()
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .map_err(|e| e.to_string())
}

async fn authorize_with_oauth(
    provider: OAuthProviderConfig,
) -> Result<
    (
        serde_json::Value,
        String,
        String,
        chrono::DateTime<chrono::Utc>,
    ),
    String,
> {
    let verifier = pkce_verifier();
    let challenge = pkce_challenge(&verifier);
    let state = pkce_verifier();
    let (listener, redirect_uri) = bind_oauth_listener(provider)?;

    let mut auth_url = reqwest::Url::parse(provider.authorize_url).map_err(|e| e.to_string())?;
    {
        let mut query = auth_url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", provider.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", provider.scope)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        for (key, value) in provider.auth_extra_params {
            query.append_pair(key, value);
        }
    }

    open::that(auth_url.as_str()).map_err(|e| format!("Could not open browser: {e}"))?;

    let callback_path = provider.callback_path.to_string();
    let callback_state = state.clone();
    let code = tauri::async_runtime::spawn_blocking(move || {
        wait_for_callback(listener, callback_path, callback_state)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e)?;

    let response = {
        let code = code.clone();
        let verifier = verifier.clone();
        let redirect_uri = redirect_uri.clone();
        let state = state.clone();
        tauri::async_runtime::spawn_blocking(move || {
            exchange_code(provider, &code, &verifier, &redirect_uri, &state)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e)?
    };

    let (access, refresh, expires_at) = oauth_tokens_from_response(&response)?;
    Ok((response, access, refresh, expires_at))
}

#[tauri::command]
async fn connect_openai_oauth(window: WebviewWindow, app: AppHandle) -> Result<String, String> {
    require_window_label(&window, SETTINGS_LABEL, "connect_openai_oauth")?;
    let (_, access, refresh, expires_at) = authorize_with_oauth(OPENAI_OAUTH_PROVIDER).await?;

    usageguard_core::set_openai_oauth_tokens(&access, expires_at, &refresh, "", "")
        .map_err(|e| e.to_string())?;

    let plan_type = tauri::async_runtime::spawn_blocking(usageguard_core::fetch_openai_oauth_usage)
        .await
        .ok()
        .flatten()
        .and_then(|_| usageguard_core::get_openai_oauth_plan_type())
        .unwrap_or_else(|| "connected".to_string());

    emit_widget_refresh(&app);
    Ok(plan_type)
}

#[tauri::command]
async fn connect_anthropic_oauth(window: WebviewWindow, app: AppHandle) -> Result<String, String> {
    require_window_label(&window, SETTINGS_LABEL, "connect_anthropic_oauth")?;
    let (response, access, refresh, expires_at) =
        authorize_with_oauth(ANTHROPIC_OAUTH_PROVIDER).await?;

    let subscription_type = oauth_response_string(
        &response,
        &[
            "subscriptionType",
            "subscription_type",
            "planType",
            "plan_type",
        ],
    )
    .unwrap_or_default();
    let rate_limit_tier =
        oauth_response_string(&response, &["rateLimitTier", "rate_limit_tier", "tier"])
            .unwrap_or_default();

    usageguard_core::set_anthropic_oauth_tokens(
        &access,
        expires_at,
        &refresh,
        &subscription_type,
        &rate_limit_tier,
    )
    .map_err(|e| e.to_string())?;

    let plan_type =
        tauri::async_runtime::spawn_blocking(usageguard_core::fetch_anthropic_oauth_usage)
            .await
            .ok()
            .flatten()
            .and_then(|_| usageguard_core::get_anthropic_oauth_plan_type())
            .unwrap_or_else(|| "connected".to_string());

    emit_widget_refresh(&app);
    Ok(plan_type)
}

#[tauri::command]
fn disconnect_openai_oauth(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "disconnect_openai_oauth")?;
    usageguard_core::clear_openai_oauth_tokens();
    emit_widget_refresh(&app);
    Ok(())
}

#[tauri::command]
fn disconnect_anthropic_oauth(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "disconnect_anthropic_oauth")?;
    usageguard_core::clear_anthropic_oauth_tokens();
    emit_widget_refresh(&app);
    Ok(())
}

#[derive(Serialize)]
struct OAuthStatus {
    connected: bool,
    plan_type: Option<String>,
}

fn oauth_status(connected: bool, plan_type: Option<String>) -> OAuthStatus {
    OAuthStatus {
        connected,
        plan_type,
    }
}

#[tauri::command]
fn get_openai_oauth_status() -> OAuthStatus {
    let connected = usageguard_core::has_openai_oauth_session();
    let plan_type = if connected {
        usageguard_core::get_openai_oauth_plan_type().filter(|s| !s.is_empty())
    } else {
        None
    };
    oauth_status(connected, plan_type)
}

#[tauri::command]
fn get_anthropic_oauth_status() -> OAuthStatus {
    let connected = usageguard_core::has_anthropic_oauth_session();
    let plan_type = if connected {
        usageguard_core::get_anthropic_oauth_plan_type().filter(|s| !s.is_empty())
    } else {
        None
    };
    oauth_status(connected, plan_type)
}

#[cfg(test)]
mod tests {
    use super::{parse_callback_code, ANTHROPIC_OAUTH_PROVIDER, OPENAI_OAUTH_PROVIDER};

    #[test]
    fn callback_requires_matching_state() {
        let err = parse_callback_code(
            "/auth/callback?code=test&state=wrong",
            OPENAI_OAUTH_PROVIDER.callback_path,
            "expected",
        )
        .unwrap_err();
        assert!(err.contains("state mismatch"));
    }

    #[test]
    fn callback_requires_expected_path() {
        let err = parse_callback_code(
            "/other?code=test&state=expected",
            OPENAI_OAUTH_PROVIDER.callback_path,
            "expected",
        )
        .unwrap_err();
        assert!(err.contains("Unexpected callback path"));
    }

    #[test]
    fn callback_extracts_code() {
        let code = parse_callback_code(
            "/auth/callback?code=test-code&state=expected",
            OPENAI_OAUTH_PROVIDER.callback_path,
            "expected",
        )
        .unwrap();
        assert_eq!(code, "test-code");
    }

    #[test]
    fn anthropic_callback_extracts_code() {
        let code = parse_callback_code(
            "/callback?code=claude-code&state=expected",
            ANTHROPIC_OAUTH_PROVIDER.callback_path,
            "expected",
        )
        .unwrap();
        assert_eq!(code, "claude-code");
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let cfg = load_config().unwrap_or_default();
            let saved_position = cfg.widget_position;
            app.manage(AppState {
                cfg: Mutex::new(cfg),
                last_notified: Mutex::new(HashMap::new()),
            });

            // Restore last widget position, or default to bottom-right of the work area.
            if let Some(win) = app.get_webview_window("main") {
                if let Some([x, y]) = saved_position {
                    let _ = win.set_position(tauri::LogicalPosition::new(x, y));
                } else if let Ok(Some(monitor)) = win.current_monitor() {
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
                    let margin_right = 30.0;
                    let margin_bottom = 14.0;
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
            update_config,
            quit,
            show_context_menu,
            set_window_rect,
            connect_openai_oauth,
            connect_anthropic_oauth,
            disconnect_openai_oauth,
            disconnect_anthropic_oauth,
            get_openai_oauth_status,
            get_anthropic_oauth_status,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
