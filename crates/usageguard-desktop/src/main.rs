#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon_art;

use chrono::{Local, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use usageguard_core::{
    clamp_refresh_interval_secs, evaluate_alerts, load_config, provider_snapshots, save_config,
    should_notify_alert, Alert, AppConfig, UsageSnapshot, MAX_REFRESH_INTERVAL_SECS,
    MIN_REFRESH_INTERVAL_SECS,
};

#[derive(Default)]
struct RefreshState {
    in_flight: bool,
    queued: bool,
}

#[derive(Default)]
struct ClaudeBootstrapState {
    attempted: bool,
    in_flight: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SnapshotView {
    #[serde(flatten)]
    snapshot: UsageSnapshot,
    #[serde(default)]
    alerts: Vec<Alert>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AlertSignature {
    code: String,
    cycle_key: String,
}

#[derive(Debug, Clone)]
struct PendingAlertNotification {
    signature: AlertSignature,
    body: String,
}

#[derive(Debug, Clone)]
struct ManualAlert {
    alert: Alert,
    expires_at: Instant,
}

struct AppState {
    cfg: Mutex<AppConfig>,
    /// Key: `"provider::account_label"`.
    /// Value: the last non-empty set of emitted alert signatures for that card.
    notified_alerts: Mutex<HashMap<String, HashMap<String, String>>>,
    snapshots: Mutex<Vec<SnapshotView>>,
    manual_alerts: Mutex<HashMap<String, ManualAlert>>,
    refresh: Mutex<RefreshState>,
    claude_bootstrap: Mutex<ClaudeBootstrapState>,
    tray_available: Mutex<bool>,
    start_on_login_enabled: Mutex<bool>,
    tray_start_on_login_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
}

const TRAY_TOGGLE_ID: &str = "tray.toggle";

const TRAY_PROVIDERS_ID: &str = "tray.providers";

const TRAY_START_ON_LOGIN_ID: &str = "tray.start_on_login";

const TRAY_QUIT_ID: &str = "tray.quit";

const CTX_REFRESH_ID: &str = "widget.refresh";

const CTX_PROVIDERS_ID: &str = "widget.providers";

const CTX_START_ON_LOGIN_ID: &str = "widget.start_on_login";

const CTX_ALWAYS_ON_TOP_ID: &str = "widget.always_on_top";

const CTX_LIGHT_MODE_ID: &str = "widget.light_mode";

const CTX_HIDE_ID: &str = "widget.hide";

const CTX_QUIT_ID: &str = "widget.quit";

const REFRESH_EVENT: &str = "usageguard://refresh";

const SETTINGS_LABEL: &str = "settings";

const CONSUMER_LOCAL_STATUS_SOURCE: &str = "consumer_local_status";

const CONSUMER_LOCAL_WAITING_FOR_USAGE_CODE: &str = "consumer_local_waiting_for_usage";

const CONSUMER_LOCAL_USAGE_PENDING_CODE: &str = "consumer_local_usage_pending";

const CLAUDE_AUTO_BOOTSTRAP_PROMPTS: [&str; 2] = ["Respond with exactly OK.", "/insights"];

const RELEASES_LATEST_URL: &str =
    "https://api.github.com/repos/openchert/usage-guard/releases/latest";

const RELEASE_CHECK_TITLE: &str = "UsageGuard update available";

const TEST_ALERT_CODE: &str = "manual_test_alert";

const TEST_ALERT_MESSAGE: &str = "Test alert: notifications and widget badges are working.";

const TEST_ALERT_DURATION: Duration = Duration::from_secs(10);

const START_ON_LOGIN_LABEL: &str = "Start on Login";

const START_ON_LOGIN_FAILURE_MESSAGE: &str = "Could not update Start on Login.";

#[cfg(target_os = "windows")]
const WINDOWS_RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
const WINDOWS_RUN_VALUE_NAME: &str = "UsageGuard";

#[cfg(target_os = "linux")]
const LINUX_AUTOSTART_FILE_NAME: &str = "com.usageguard.app.desktop";

#[cfg(target_os = "linux")]
const XDG_CONFIG_HOME_ENV: &str = "XDG_CONFIG_HOME";

const DEFAULT_WIDGET_WIDTH: f64 = 244.0;

const DEFAULT_WIDGET_HEIGHT: f64 = 100.0;

const DEFAULT_WIDGET_MARGIN_RIGHT: f64 = 30.0;

const DEFAULT_WIDGET_MARGIN_BOTTOM: f64 = 14.0;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TestAlertInput {
    provider: String,
    account_label: String,
}

#[tauri::command]
fn get_snapshots(state: State<AppState>) -> Vec<SnapshotView> {
    state
        .snapshots
        .lock()
        .expect("AppState snapshots lock poisoned")
        .clone()
}

#[tauri::command]
fn get_config(state: State<AppState>) -> AppConfig {
    state
        .cfg
        .lock()
        .expect("AppState cfg lock poisoned")
        .clone()
}

#[tauri::command]
fn get_refresh_interval_secs(state: State<AppState>) -> u32 {
    state
        .cfg
        .lock()
        .expect("AppState cfg lock poisoned")
        .refresh_interval_secs
}

#[tauri::command]
fn refresh_snapshots(app: AppHandle) {
    spawn_snapshot_refresh(app);
}

fn validate_refresh_interval_secs(refresh_interval_secs: u32) -> Result<u32, String> {
    let normalized = clamp_refresh_interval_secs(refresh_interval_secs);

    if normalized != refresh_interval_secs {
        return Err(format!(
            "Refresh interval must be between {MIN_REFRESH_INTERVAL_SECS} and {MAX_REFRESH_INTERVAL_SECS} seconds."
        ));
    }

    Ok(refresh_interval_secs)
}

#[tauri::command]
fn set_refresh_interval_secs(
    window: WebviewWindow,
    refresh_interval_secs: u32,
    state: State<AppState>,
    app: AppHandle,
) -> Result<u32, String> {
    require_window_label(&window, SETTINGS_LABEL, "set_refresh_interval_secs")?;

    let refresh_interval_secs = validate_refresh_interval_secs(refresh_interval_secs)?;

    let mut guard = state.cfg.lock().expect("AppState cfg lock poisoned");

    guard.refresh_interval_secs = refresh_interval_secs;

    save_config(&guard).map_err(|error| error.to_string())?;

    drop(guard);

    emit_widget_refresh(&app);

    Ok(refresh_interval_secs)
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

fn tray_available(app: &AppHandle) -> bool {
    *app.state::<AppState>()
        .tray_available
        .lock()
        .expect("AppState tray_available lock poisoned")
}

fn set_tray_available(app: &AppHandle, available: bool) {
    *app.state::<AppState>()
        .tray_available
        .lock()
        .expect("AppState tray_available lock poisoned") = available;
}

fn emit_widget_refresh(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.emit(REFRESH_EVENT, ());
    }

    if let Some(win) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = win.emit(REFRESH_EVENT, ());
    }
}

fn snapshot_key(snapshot: &UsageSnapshot) -> String {
    format!("{}::{}", snapshot.provider, snapshot.account_label)
}

fn prune_manual_alerts(manual_alerts: &mut HashMap<String, ManualAlert>) {
    let now = Instant::now();

    manual_alerts.retain(|_, manual_alert| manual_alert.expires_at > now);
}

fn active_manual_alerts(state: &AppState) -> HashMap<String, ManualAlert> {
    let mut manual_alerts = state
        .manual_alerts
        .lock()
        .expect("AppState manual_alerts lock poisoned");

    prune_manual_alerts(&mut manual_alerts);

    manual_alerts.clone()
}

fn apply_manual_alerts(
    snapshot_views: &mut [SnapshotView],
    manual_alerts: &HashMap<String, ManualAlert>,
) {
    for snapshot_view in snapshot_views.iter_mut() {
        if let Some(manual_alert) = manual_alerts.get(&snapshot_key(&snapshot_view.snapshot)) {
            snapshot_view
                .alerts
                .retain(|alert| alert.code != manual_alert.alert.code);

            snapshot_view.alerts.push(manual_alert.alert.clone());
        }
    }
}

fn find_snapshot_for_test_alert(
    state: &AppState,
    target: &TestAlertInput,
) -> Option<UsageSnapshot> {
    state
        .snapshots
        .lock()
        .expect("AppState snapshots lock poisoned")
        .iter()
        .find(|view| {
            view.snapshot.provider == target.provider
                && view.snapshot.account_label == target.account_label
        })
        .map(|view| view.snapshot.clone())
}

fn spawn_manual_alert_expiry(app: AppHandle, target_key: String, expires_at: Instant) {
    std::thread::spawn(move || {
        if let Some(delay) = expires_at.checked_duration_since(Instant::now()) {
            std::thread::sleep(delay);
        }

        let state = app.state::<AppState>();

        let removed = {
            let mut manual_alerts = state
                .manual_alerts
                .lock()
                .expect("AppState manual_alerts lock poisoned");

            prune_manual_alerts(&mut manual_alerts);

            let is_current = manual_alerts
                .get(&target_key)
                .map(|manual_alert| manual_alert.expires_at == expires_at)
                .unwrap_or(false);

            if is_current {
                manual_alerts.remove(&target_key);

                true
            } else {
                false
            }
        };

        if !removed {
            return;
        }

        {
            let mut snapshots = state
                .snapshots
                .lock()
                .expect("AppState snapshots lock poisoned");

            for snapshot_view in snapshots
                .iter_mut()
                .filter(|view| snapshot_key(&view.snapshot) == target_key)
            {
                snapshot_view
                    .alerts
                    .retain(|alert| alert.code != TEST_ALERT_CODE);
            }
        }

        emit_widget_refresh(&app);
    });
}

fn snapshot_views(
    snapshots: &[UsageSnapshot],
    now: chrono::DateTime<Utc>,
    cfg: &AppConfig,
) -> Vec<SnapshotView> {
    snapshots
        .iter()
        .cloned()
        .map(|snapshot| SnapshotView {
            alerts: evaluate_alerts(&snapshot, now, cfg),
            snapshot,
        })
        .collect()
}

fn refresh_snapshot_alert_state(state: &AppState, cfg: &AppConfig) {
    let snapshots = state
        .snapshots
        .lock()
        .expect("AppState snapshots lock poisoned")
        .iter()
        .map(|view| view.snapshot.clone())
        .collect::<Vec<_>>();

    let manual_alerts = active_manual_alerts(state);

    let mut refreshed = snapshot_views(&snapshots, Utc::now(), cfg);

    apply_manual_alerts(&mut refreshed, &manual_alerts);

    *state
        .snapshots
        .lock()
        .expect("AppState snapshots lock poisoned") = refreshed;
}

fn spawn_snapshot_refresh(app: AppHandle) {
    let should_spawn = {
        let state = app.state::<AppState>();

        let mut refresh = state
            .refresh
            .lock()
            .expect("AppState refresh lock poisoned");

        if refresh.in_flight {
            refresh.queued = true;

            false
        } else {
            refresh.in_flight = true;

            refresh.queued = false;

            true
        }
    };

    if !should_spawn {
        return;
    }

    std::thread::spawn(move || loop {
        let state = app.state::<AppState>();

        let cfg = state
            .cfg
            .lock()
            .expect("AppState cfg lock poisoned")
            .clone();

        let snapshots = provider_snapshots(&cfg);

        maybe_spawn_claude_auto_bootstrap(&app, &snapshots);

        let now_local = Local::now();

        let now_utc = now_local.with_timezone(&Utc);

        let manual_alerts = active_manual_alerts(state.inner());

        let mut snapshot_views = snapshot_views(&snapshots, now_utc, &cfg);

        apply_manual_alerts(&mut snapshot_views, &manual_alerts);

        {
            let mut cache = state
                .snapshots
                .lock()
                .expect("AppState snapshots lock poisoned");

            *cache = snapshot_views.clone();
        }

        fire_notifications(
            &snapshot_views,
            now_local,
            &cfg,
            &mut state
                .notified_alerts
                .lock()
                .expect("AppState notified_alerts lock poisoned"),
        );

        emit_widget_refresh(&app);

        let should_continue = {
            let mut refresh = state
                .refresh
                .lock()
                .expect("AppState refresh lock poisoned");

            if refresh.queued {
                refresh.queued = false;

                true
            } else {
                refresh.in_flight = false;

                false
            }
        };

        if !should_continue {
            break;
        }
    });
}

fn anthropic_snapshot_needs_bootstrap(snapshot: &UsageSnapshot) -> bool {
    if snapshot.provider != "anthropic" || snapshot.source != CONSUMER_LOCAL_STATUS_SOURCE {
        return false;
    }

    if snapshot.consumer_quota.is_some() {
        return false;
    }

    snapshot.status_code.as_deref().is_some_and(|code| {
        code == CONSUMER_LOCAL_WAITING_FOR_USAGE_CODE || code == CONSUMER_LOCAL_USAGE_PENDING_CODE
    })
}

fn maybe_spawn_claude_auto_bootstrap(app: &AppHandle, snapshots: &[UsageSnapshot]) {
    if !snapshots.iter().any(anthropic_snapshot_needs_bootstrap) {
        return;
    }

    let should_spawn = {
        let state = app.state::<AppState>();

        let mut bootstrap = state
            .claude_bootstrap
            .lock()
            .expect("AppState claude_bootstrap lock poisoned");

        if bootstrap.attempted || bootstrap.in_flight {
            false
        } else {
            bootstrap.attempted = true;
            bootstrap.in_flight = true;
            true
        }
    };

    if !should_spawn {
        return;
    }

    let app_handle = app.clone();

    std::thread::spawn(move || {
        for prompt in CLAUDE_AUTO_BOOTSTRAP_PROMPTS {
            let command_result = std::process::Command::new("claude")
                .args(["-p", prompt])
                .output();

            match command_result {
                Ok(output) if output.status.success() => {
                    break;
                }
                Ok(output) => {
                    let _ = (prompt, output);
                }
                Err(error) => {
                    let _ = (prompt, error);
                }
            }
        }

        usageguard_core::invalidate_claude_local_insights_cache();

        {
            let state = app_handle.state::<AppState>();

            let mut bootstrap = state
                .claude_bootstrap
                .lock()
                .expect("AppState claude_bootstrap lock poisoned");

            bootstrap.in_flight = false;
        }

        spawn_snapshot_refresh(app_handle);
    });
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct LogicalWorkArea {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

fn logical_work_area(monitor: &tauri::Monitor) -> LogicalWorkArea {
    let scale = monitor.scale_factor();

    let work_area = monitor.work_area();

    let left = work_area.position.x as f64 / scale;

    let top = work_area.position.y as f64 / scale;

    let width = work_area.size.width as f64 / scale;

    let height = work_area.size.height as f64 / scale;

    LogicalWorkArea {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}

fn work_area_contains_point(area: LogicalWorkArea, x: f64, y: f64) -> bool {
    x >= area.left && x <= area.right && y >= area.top && y <= area.bottom
}

fn clamp_widget_origin_to_area(area: LogicalWorkArea, x: f64, y: f64) -> (f64, f64) {
    let max_x = (area.right - DEFAULT_WIDGET_WIDTH).max(area.left);

    let max_y = (area.bottom - DEFAULT_WIDGET_HEIGHT).max(area.top);

    (x.clamp(area.left, max_x), y.clamp(area.top, max_y))
}

fn clamp_popup_origin_to_area(
    area: LogicalWorkArea,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> (f64, f64) {
    let max_x = (area.right - width).max(area.left);

    let max_y = (area.bottom - height).max(area.top);

    (x.clamp(area.left, max_x), y.clamp(area.top, max_y))
}

fn default_widget_origin_for_area(area: LogicalWorkArea) -> (f64, f64) {
    clamp_widget_origin_to_area(
        area,
        area.right - DEFAULT_WIDGET_WIDTH - DEFAULT_WIDGET_MARGIN_RIGHT,
        area.bottom - DEFAULT_WIDGET_HEIGHT - DEFAULT_WIDGET_MARGIN_BOTTOM,
    )
}

fn preferred_widget_work_area(win: &WebviewWindow) -> Option<LogicalWorkArea> {
    win.current_monitor()
        .ok()
        .flatten()
        .or_else(|| win.primary_monitor().ok().flatten())
        .or_else(|| {
            win.available_monitors()
                .ok()
                .and_then(|monitors| monitors.into_iter().next())
        })
        .map(|monitor| logical_work_area(&monitor))
}

fn restored_widget_origin(
    win: &WebviewWindow,
    saved_position: Option<[f64; 2]>,
) -> Option<(f64, f64)> {
    if let Some([right, bottom]) = saved_position {
        if let Ok(monitors) = win.available_monitors() {
            if let Some(area) = monitors
                .into_iter()
                .map(|monitor| logical_work_area(&monitor))
                .find(|area| work_area_contains_point(*area, right, bottom))
            {
                return Some(clamp_widget_origin_to_area(
                    area,
                    right - DEFAULT_WIDGET_WIDTH,
                    bottom - DEFAULT_WIDGET_HEIGHT,
                ));
            }
        }
    }

    preferred_widget_work_area(win).map(default_widget_origin_for_area)
}

fn restore_widget_position(win: &WebviewWindow, saved_position: Option<[f64; 2]>) {
    if let Some((x, y)) = restored_widget_origin(win, saved_position) {
        let _ = win.set_position(tauri::LogicalPosition::new(x, y));
    }
}

fn adjacent_popup_position(
    main_win: &WebviewWindow,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    const GAP: f64 = 8.0;
    const BOTTOM_ALIGNMENT_OFFSET: f64 = 8.0;

    let scale = main_win.scale_factor().ok()?;

    let phys_pos = main_win.outer_position().ok()?;

    let phys_size = main_win.inner_size().ok()?;

    let widget_x = phys_pos.x as f64 / scale;
    let widget_y = phys_pos.y as f64 / scale;
    let widget_w = phys_size.width as f64 / scale;
    let widget_h = phys_size.height as f64 / scale;

    let area = preferred_widget_work_area(main_win)?;

    let left_x = widget_x - width - GAP;
    let right_x = widget_x + widget_w + GAP;
    let preferred_x = if left_x >= area.left {
        left_x
    } else if right_x + width <= area.right {
        right_x
    } else {
        left_x
    };

    let preferred_y = widget_y + widget_h - height + BOTTOM_ALIGNMENT_OFFSET;

    Some(clamp_popup_origin_to_area(
        area,
        preferred_x,
        preferred_y,
        width,
        height,
    ))
}

fn open_provider_settings_impl(app: &AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window(SETTINGS_LABEL) {
        let _ = win.set_always_on_top(true);

        let _ = win.show();

        let _ = win.set_focus();

        return Ok(());
    }

    const SETTINGS_W: f64 = 360.0;

    const SETTINGS_H: f64 = 348.0;

    let position = app
        .get_webview_window("main")
        .and_then(|main_win| adjacent_popup_position(&main_win, SETTINGS_W, SETTINGS_H));

    let builder = WebviewWindowBuilder::new(
        app,
        SETTINGS_LABEL,
        WebviewUrl::App("index.html?view=settings".into()),
    )
    .title("UsageGuard Connections")
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
        let _ = open_provider_settings_impl(&app);
    });
}

#[tauri::command]
fn open_provider_settings(app: AppHandle) {
    spawn_open_provider_settings(app);
}

/// Saves the current widget position to config, then exits.
/// Called from every quit path so the position is always persisted.
/// We save the right-bottom corner (not left-top) so that resizeToFit, which
/// anchors the widget to its right-bottom edge, correctly restores the position
/// regardless of how many provider cards are shown on next launch.

fn save_position_and_exit(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        if let (Ok(pos), Ok(size), Ok(scale)) =
            (win.outer_position(), win.inner_size(), win.scale_factor())
        {
            let right = (pos.x as f64 + size.width as f64) / scale;

            let bottom = (pos.y as f64 + size.height as f64) / scale;

            let state = app.state::<AppState>();

            let mut guard = state.cfg.lock().expect("AppState cfg lock poisoned");

            guard.widget_position = Some([right, bottom]);

            let _ = save_config(&guard);
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

    let result = window
        .popup_menu_at(&menu, tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string());

    #[cfg(target_os = "windows")]
    flush_context_menu(&window);

    result
}

#[cfg(target_os = "windows")]
fn flush_context_menu(window: &WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            // Win32 popup menus can linger visually unless the owner window
            // receives a follow-up message after TrackPopupMenu returns.
            win32::PostMessageW(hwnd.0 as isize, win32::WM_NULL, 0, 0);
        }
    }
}

/// Inline FFI bindings — no external crate needed, user32.dll is always present.
#[cfg(target_os = "windows")]
mod win32 {

    pub const SWP_NOACTIVATE: u32 = 0x0010;

    pub const SWP_NOZORDER: u32 = 0x0004;

    pub const WM_NULL: u32 = 0x0000;

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

        pub fn PostMessageW(hwnd: isize, msg: u32, w_param: usize, l_param: isize) -> i32;

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
    snapshots: &[SnapshotView],
    now: chrono::DateTime<Local>,
    cfg: &AppConfig,
    notified_alerts: &mut HashMap<String, HashMap<String, String>>,
) {
    for (title, body) in collect_pending_notifications(snapshots, now, cfg, notified_alerts) {
        emit_native_notification(&title, &body);
    }
}

fn collect_pending_notifications(
    snapshots: &[SnapshotView],
    now: chrono::DateTime<Local>,
    cfg: &AppConfig,
    notified_alerts: &mut HashMap<String, HashMap<String, String>>,
) -> Vec<(String, String)> {
    let mut pending = Vec::new();

    for snapshot_view in snapshots {
        let remembered = notified_alerts.get(&snapshot_key(&snapshot_view.snapshot));
        let mut current = Vec::new();

        for alert in &snapshot_view.alerts {
            if should_notify_alert(alert, now, cfg) {
                current.push(PendingAlertNotification {
                    signature: alert_signature(&snapshot_view.snapshot, alert),
                    body: format!(
                        "{}: {}",
                        snapshot_view.snapshot.account_label, alert.message
                    ),
                });
            }
        }
        if current.is_empty() {
            continue;
        }

        for notification in current.iter().filter(|notification| {
            !remembered_contains_signature(remembered, &notification.signature)
        }) {
            pending.push(("UsageGuard".to_string(), notification.body.clone()));
        }

        remember_snapshot_alerts(notified_alerts, &snapshot_view.snapshot, &current);
    }

    pending
}

fn remembered_contains_signature(
    remembered: Option<&HashMap<String, String>>,
    signature: &AlertSignature,
) -> bool {
    remembered
        .and_then(|remembered| remembered.get(&signature.code))
        .is_some_and(|previous| alert_cycles_match(previous, &signature.cycle_key))
}

fn is_stable_cycle_key(cycle_key: &str) -> bool {
    cycle_key == "stable" || cycle_key.ends_with("-stable")
}

fn alert_cycles_match(left: &str, right: &str) -> bool {
    left == right || is_stable_cycle_key(left) || is_stable_cycle_key(right)
}

fn preferred_cycle_key(previous: &str, current: &str) -> String {
    match (is_stable_cycle_key(previous), is_stable_cycle_key(current)) {
        (true, false) => current.to_string(),
        (false, true) => previous.to_string(),
        _ => current.to_string(),
    }
}

fn remember_snapshot_alerts(
    notified_alerts: &mut HashMap<String, HashMap<String, String>>,
    snapshot: &UsageSnapshot,
    current: &[PendingAlertNotification],
) {
    let snapshot_id = snapshot_key(snapshot);
    let previous = notified_alerts.remove(&snapshot_id).unwrap_or_default();
    let mut remembered = HashMap::with_capacity(current.len());

    for notification in current {
        let cycle_key = previous
            .get(&notification.signature.code)
            .filter(|previous| alert_cycles_match(previous, &notification.signature.cycle_key))
            .map(|previous| preferred_cycle_key(previous, &notification.signature.cycle_key))
            .unwrap_or_else(|| notification.signature.cycle_key.clone());

        remembered.insert(notification.signature.code.clone(), cycle_key);
    }

    notified_alerts.insert(snapshot_id, remembered);
}

fn remember_emitted_alert(
    notified_alerts: &mut HashMap<String, HashMap<String, String>>,
    snapshot: &UsageSnapshot,
    alert: &Alert,
) {
    let signature = alert_signature(snapshot, alert);
    let snapshot_alerts = notified_alerts.entry(snapshot_key(snapshot)).or_default();
    let cycle_key = snapshot_alerts
        .get(&signature.code)
        .map(|previous| preferred_cycle_key(previous, &signature.cycle_key))
        .unwrap_or_else(|| signature.cycle_key.clone());

    snapshot_alerts.insert(signature.code, cycle_key);
}

fn canonicalize_cycle_key(value: &str) -> String {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return "stable".to_string();
    }

    chrono::DateTime::parse_from_rfc3339(trimmed)
        .map(|parsed| {
            parsed
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| trimmed.to_string())
}

fn alert_signature(snapshot: &UsageSnapshot, alert: &Alert) -> AlertSignature {
    AlertSignature {
        code: alert.code.clone(),
        cycle_key: alert_cycle_key(snapshot, alert),
    }
}

/// Returns the "cycle discriminator" for an alert. This changes when the
/// underlying quota window genuinely resets, which re-arms notification delivery.

fn alert_cycle_key(snapshot: &UsageSnapshot, alert: &Alert) -> String {
    match alert.code.as_str() {
        "quota_5h_exhausted" | "quota_5h_near_limit" | "quota_5h_unused_before_reset" => snapshot
            .primary_reset_at
            .as_deref()
            .map(canonicalize_cycle_key)
            .unwrap_or_else(|| "5h-stable".to_string()),
        "quota_week_exhausted" | "quota_week_near_limit" | "quota_week_unused_before_reset" => {
            snapshot
                .secondary_reset_at
                .as_deref()
                .map(canonicalize_cycle_key)
                .unwrap_or_else(|| "week-stable".to_string())
        }
        _ => "stable".to_string(),
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

#[derive(Debug, Deserialize)]
struct LatestReleaseResponse {
    tag_name: String,
}

#[cfg(target_os = "windows")]
fn release_update_message(latest_tag: &str) -> String {
    format!(
        "{latest_tag} is available. Re-run the installer or download the latest release to update."
    )
}

#[cfg(target_os = "linux")]
fn release_update_message(latest_tag: &str) -> String {
    format!(
        "{latest_tag} is available. Download the latest .deb or .AppImage from GitHub Releases to update."
    )
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn release_update_message(latest_tag: &str) -> String {
    format!("{latest_tag} is available. Download the latest release to update.")
}

fn spawn_release_check(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }

    std::thread::spawn(move || {
        let latest_tag = match fetch_latest_release_tag() {
            Ok(tag_name) => tag_name,
            Err(error) => {
                let _ = error;
                return;
            }
        };

        if compare_versions(&latest_tag, env!("CARGO_PKG_VERSION")) != Some(Ordering::Greater) {
            return;
        }

        let should_notify = {
            let state = app.state::<AppState>();

            let mut cfg = state.cfg.lock().expect("AppState cfg lock poisoned");

            if cfg.last_update_notified_version.as_deref() == Some(latest_tag.as_str()) {
                false
            } else {
                cfg.last_update_notified_version = Some(latest_tag.clone());

                let _ = save_config(&cfg);

                true
            }
        };

        if should_notify {
            emit_native_notification(RELEASE_CHECK_TITLE, &release_update_message(&latest_tag));
        }
    });
}

fn fetch_latest_release_tag() -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(4))
        .build()
        .map_err(|error| error.to_string())?;

    let response = client
        .get(RELEASES_LATEST_URL)
        .header(
            reqwest::header::USER_AGENT,
            format!("usageguard-desktop/{}", env!("CARGO_PKG_VERSION")),
        )
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| error.to_string())?;

    let release = response
        .json::<LatestReleaseResponse>()
        .map_err(|error| error.to_string())?;

    Ok(release.tag_name)
}

fn compare_versions(left: &str, right: &str) -> Option<Ordering> {
    let left_parts = parse_version_parts(left)?;

    let right_parts = parse_version_parts(right)?;

    let len = left_parts.len().max(right_parts.len());

    for index in 0..len {
        let left = *left_parts.get(index).unwrap_or(&0);

        let right = *right_parts.get(index).unwrap_or(&0);

        match left.cmp(&right) {
            Ordering::Equal => continue,
            non_equal => return Some(non_equal),
        }
    }

    Some(Ordering::Equal)
}

fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let core = version
        .trim()
        .trim_start_matches('v')
        .split(|ch| ch == '-' || ch == '+')
        .next()?;

    let parts = core
        .split('.')
        .map(|part| part.parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;

    (!parts.is_empty()).then_some(parts)
}

fn create_tray_icon() -> tauri::image::Image<'static> {
    static PIXELS: OnceLock<Box<[u8]>> = OnceLock::new();

    let size = icon_art::TRAY_ICON_SIZE;

    let data = PIXELS.get_or_init(|| icon_art::icon_rgba_pixels(size).into_boxed_slice());

    tauri::image::Image::new(data, size, size)
}

fn current_executable_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|error| format!("failed to resolve current executable: {error}"))?;

    if exe.is_absolute() {
        Ok(exe)
    } else {
        let cwd = std::env::current_dir()
            .map_err(|error| format!("failed to resolve current directory: {error}"))?;

        Ok(cwd.join(exe))
    }
}

#[cfg(target_os = "windows")]
fn windows_start_on_login_command() -> Result<String, String> {
    let exe = current_executable_path()?;

    Ok(format!("\"{}\"", exe.display()))
}

#[cfg(target_os = "windows")]
fn run_reg_command(args: &[&str]) -> Result<std::process::Output, String> {
    std::process::Command::new("reg")
        .args(args)
        .output()
        .map_err(|error| format!("failed to run reg.exe: {error}"))
}

#[cfg(target_os = "windows")]
fn reg_command_error(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !stderr.is_empty() {
        return stderr;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if !stdout.is_empty() {
        stdout
    } else {
        format!("exit status {}", output.status)
    }
}

#[cfg(target_os = "windows")]
fn is_start_on_login_enabled() -> bool {
    run_reg_command(&["query", WINDOWS_RUN_KEY, "/v", WINDOWS_RUN_VALUE_NAME])
        .map(|output| output.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn linux_autostart_dir() -> Result<PathBuf, String> {
    if let Some(config_home) =
        std::env::var_os(XDG_CONFIG_HOME_ENV).filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(config_home).join("autostart"));
    }

    let home =
        dirs::home_dir().ok_or_else(|| "failed to resolve HOME for autostart".to_string())?;

    Ok(home.join(".config").join("autostart"))
}

#[cfg(target_os = "linux")]
fn linux_autostart_path() -> Result<PathBuf, String> {
    Ok(linux_autostart_dir()?.join(LINUX_AUTOSTART_FILE_NAME))
}

#[cfg(target_os = "linux")]
fn desktop_exec_value(path: &std::path::Path) -> String {
    let escaped = path
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");

    format!("\"{escaped}\"")
}

#[cfg(target_os = "linux")]
fn linux_autostart_file_contents() -> Result<String, String> {
    let exe = current_executable_path()?;

    Ok(format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName=UsageGuard\nExec={}\nTerminal=false\n",
        desktop_exec_value(&exe)
    ))
}

#[cfg(target_os = "linux")]
fn is_start_on_login_enabled() -> bool {
    linux_autostart_path()
        .map(|path| path.is_file())
        .unwrap_or(false)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn is_start_on_login_enabled() -> bool {
    false
}

fn cached_start_on_login_enabled(app: &AppHandle) -> bool {
    *app.state::<AppState>()
        .start_on_login_enabled
        .lock()
        .expect("AppState start_on_login_enabled lock poisoned")
}

fn set_cached_start_on_login_enabled(app: &AppHandle, enabled: bool) {
    *app.state::<AppState>()
        .start_on_login_enabled
        .lock()
        .expect("AppState start_on_login_enabled lock poisoned") = enabled;

    if let Some(item) = app
        .state::<AppState>()
        .tray_start_on_login_item
        .lock()
        .expect("AppState tray_start_on_login_item lock poisoned")
        .as_ref()
    {
        let _ = item.set_checked(enabled);
    }
}

#[cfg(target_os = "windows")]
fn initialize_windows_start_on_login(cfg: &mut AppConfig) -> bool {
    let mut enabled = is_start_on_login_enabled();

    if cfg.windows_start_on_login_initialized {
        return enabled;
    }

    if !enabled && set_start_on_login_enabled(true).is_ok() {
        enabled = true;
    }

    if enabled {
        cfg.windows_start_on_login_initialized = true;
        let _ = save_config(cfg);
    }

    enabled
}

#[cfg(not(target_os = "windows"))]
fn initialize_windows_start_on_login(_cfg: &mut AppConfig) -> bool {
    is_start_on_login_enabled()
}

#[cfg(target_os = "windows")]
fn set_start_on_login_enabled(enabled: bool) -> Result<(), String> {
    if enabled {
        let startup_command = windows_start_on_login_command()?;

        let output = run_reg_command(&[
            "add",
            WINDOWS_RUN_KEY,
            "/v",
            WINDOWS_RUN_VALUE_NAME,
            "/t",
            "REG_SZ",
            "/d",
            startup_command.as_str(),
            "/f",
        ])?;

        if output.status.success() {
            Ok(())
        } else {
            Err(reg_command_error(&output))
        }
    } else {
        if !is_start_on_login_enabled() {
            return Ok(());
        }

        let output = run_reg_command(&[
            "delete",
            WINDOWS_RUN_KEY,
            "/v",
            WINDOWS_RUN_VALUE_NAME,
            "/f",
        ])?;

        if output.status.success() {
            Ok(())
        } else {
            Err(reg_command_error(&output))
        }
    }
}

#[cfg(target_os = "linux")]
fn set_start_on_login_enabled(enabled: bool) -> Result<(), String> {
    let path = linux_autostart_path()?;

    if enabled {
        let dir = path
            .parent()
            .ok_or_else(|| "failed to resolve autostart directory".to_string())?;

        std::fs::create_dir_all(dir)
            .map_err(|error| format!("failed to create Linux autostart directory: {error}"))?;

        std::fs::write(&path, linux_autostart_file_contents()?)
            .map_err(|error| format!("failed to write Linux autostart file: {error}"))?;
    } else if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|error| format!("failed to remove Linux autostart file: {error}"))?;
    }

    Ok(())
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn set_start_on_login_enabled(_enabled: bool) -> Result<(), String> {
    Ok(())
}

fn create_widget_menu(window: &WebviewWindow) -> tauri::Result<Menu<tauri::Wry>> {
    let app = window.app_handle();

    let always_on_top = window.is_always_on_top().unwrap_or(true);

    let light_mode = app
        .state::<AppState>()
        .cfg
        .lock()
        .expect("AppState cfg lock poisoned")
        .light_mode;

    let first_sep = PredefinedMenuItem::separator(app)?;
    let second_sep = PredefinedMenuItem::separator(app)?;
    let third_sep = PredefinedMenuItem::separator(app)?;

    #[cfg(target_os = "windows")]
    {
        let startup_enabled = cached_start_on_login_enabled(&app);

        let startup_toggle = CheckMenuItem::with_id(
            app,
            CTX_START_ON_LOGIN_ID,
            START_ON_LOGIN_LABEL,
            true,
            startup_enabled,
            None::<&str>,
        )?;

        Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
                &first_sep,
                &MenuItem::with_id(
                    app,
                    CTX_PROVIDERS_ID,
                    "Manage Connections...",
                    true,
                    None::<&str>,
                )?,
                &second_sep,
                &startup_toggle,
                &CheckMenuItem::with_id(
                    app,
                    CTX_ALWAYS_ON_TOP_ID,
                    "Always on Top",
                    true,
                    always_on_top,
                    None::<&str>,
                )?,
                &CheckMenuItem::with_id(
                    app,
                    CTX_LIGHT_MODE_ID,
                    "Light Mode",
                    true,
                    light_mode,
                    None::<&str>,
                )?,
                &MenuItem::with_id(app, CTX_HIDE_ID, "Hide to Tray", true, None::<&str>)?,
                &third_sep,
                &MenuItem::with_id(app, CTX_QUIT_ID, "Quit", true, None::<&str>)?,
            ],
        )
    }

    #[cfg(target_os = "linux")]
    {
        let startup_enabled = cached_start_on_login_enabled(&app);

        let startup_toggle = CheckMenuItem::with_id(
            app,
            CTX_START_ON_LOGIN_ID,
            START_ON_LOGIN_LABEL,
            true,
            startup_enabled,
            None::<&str>,
        )?;

        if tray_available(&app) {
            Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
                    &first_sep,
                    &MenuItem::with_id(
                        app,
                        CTX_PROVIDERS_ID,
                        "Manage Connections...",
                        true,
                        None::<&str>,
                    )?,
                    &second_sep,
                    &startup_toggle,
                    &CheckMenuItem::with_id(
                        app,
                        CTX_ALWAYS_ON_TOP_ID,
                        "Always on Top",
                        true,
                        always_on_top,
                        None::<&str>,
                    )?,
                    &CheckMenuItem::with_id(
                        app,
                        CTX_LIGHT_MODE_ID,
                        "Light Mode",
                        true,
                        light_mode,
                        None::<&str>,
                    )?,
                    &MenuItem::with_id(app, CTX_HIDE_ID, "Hide to Tray", true, None::<&str>)?,
                    &third_sep,
                    &MenuItem::with_id(app, CTX_QUIT_ID, "Quit", true, None::<&str>)?,
                ],
            )
        } else {
            Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
                    &first_sep,
                    &MenuItem::with_id(
                        app,
                        CTX_PROVIDERS_ID,
                        "Manage Connections...",
                        true,
                        None::<&str>,
                    )?,
                    &second_sep,
                    &startup_toggle,
                    &CheckMenuItem::with_id(
                        app,
                        CTX_ALWAYS_ON_TOP_ID,
                        "Always on Top",
                        true,
                        always_on_top,
                        None::<&str>,
                    )?,
                    &CheckMenuItem::with_id(
                        app,
                        CTX_LIGHT_MODE_ID,
                        "Light Mode",
                        true,
                        light_mode,
                        None::<&str>,
                    )?,
                    &third_sep,
                    &MenuItem::with_id(app, CTX_QUIT_ID, "Quit", true, None::<&str>)?,
                ],
            )
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        if tray_available(&app) {
            Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
                    &first_sep,
                    &MenuItem::with_id(
                        app,
                        CTX_PROVIDERS_ID,
                        "Manage Connections...",
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
                    &CheckMenuItem::with_id(
                        app,
                        CTX_LIGHT_MODE_ID,
                        "Light Mode",
                        true,
                        light_mode,
                        None::<&str>,
                    )?,
                    &MenuItem::with_id(app, CTX_HIDE_ID, "Hide to Tray", true, None::<&str>)?,
                    &third_sep,
                    &MenuItem::with_id(app, CTX_QUIT_ID, "Quit", true, None::<&str>)?,
                ],
            )
        } else {
            Menu::with_items(
                app,
                &[
                    &MenuItem::with_id(app, CTX_REFRESH_ID, "Refresh", true, None::<&str>)?,
                    &first_sep,
                    &MenuItem::with_id(
                        app,
                        CTX_PROVIDERS_ID,
                        "Manage Connections...",
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
                    &CheckMenuItem::with_id(
                        app,
                        CTX_LIGHT_MODE_ID,
                        "Light Mode",
                        true,
                        light_mode,
                        None::<&str>,
                    )?,
                    &third_sep,
                    &MenuItem::with_id(app, CTX_QUIT_ID, "Quit", true, None::<&str>)?,
                ],
            )
        }
    }
}

fn create_tray_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let first_sep = PredefinedMenuItem::separator(app)?;

    let second_sep = PredefinedMenuItem::separator(app)?;

    #[cfg(target_os = "windows")]
    {
        let third_sep = PredefinedMenuItem::separator(app)?;

        let startup_enabled = cached_start_on_login_enabled(app);

        let startup_toggle = CheckMenuItem::with_id(
            app,
            TRAY_START_ON_LOGIN_ID,
            START_ON_LOGIN_LABEL,
            true,
            startup_enabled,
            None::<&str>,
        )?;

        *app.state::<AppState>()
            .tray_start_on_login_item
            .lock()
            .expect("AppState tray_start_on_login_item lock poisoned") =
            Some(startup_toggle.clone());

        Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, TRAY_TOGGLE_ID, "Show / Hide", true, None::<&str>)?,
                &first_sep,
                &MenuItem::with_id(
                    app,
                    TRAY_PROVIDERS_ID,
                    "Manage Connections...",
                    true,
                    None::<&str>,
                )?,
                &second_sep,
                &startup_toggle,
                &third_sep,
                &MenuItem::with_id(app, TRAY_QUIT_ID, "Quit UsageGuard", true, None::<&str>)?,
            ],
        )
    }

    #[cfg(target_os = "linux")]
    {
        let third_sep = PredefinedMenuItem::separator(app)?;

        let startup_enabled = cached_start_on_login_enabled(app);

        let startup_toggle = CheckMenuItem::with_id(
            app,
            TRAY_START_ON_LOGIN_ID,
            START_ON_LOGIN_LABEL,
            true,
            startup_enabled,
            None::<&str>,
        )?;

        *app.state::<AppState>()
            .tray_start_on_login_item
            .lock()
            .expect("AppState tray_start_on_login_item lock poisoned") =
            Some(startup_toggle.clone());

        Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, TRAY_TOGGLE_ID, "Show / Hide", true, None::<&str>)?,
                &first_sep,
                &MenuItem::with_id(
                    app,
                    TRAY_PROVIDERS_ID,
                    "Manage Connections...",
                    true,
                    None::<&str>,
                )?,
                &second_sep,
                &startup_toggle,
                &third_sep,
                &MenuItem::with_id(app, TRAY_QUIT_ID, "Quit UsageGuard", true, None::<&str>)?,
            ],
        )
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Menu::with_items(
            app,
            &[
                &MenuItem::with_id(app, TRAY_TOGGLE_ID, "Show / Hide", true, None::<&str>)?,
                &first_sep,
                &MenuItem::with_id(
                    app,
                    TRAY_PROVIDERS_ID,
                    "Manage Connections...",
                    true,
                    None::<&str>,
                )?,
                &second_sep,
                &MenuItem::with_id(app, TRAY_QUIT_ID, "Quit UsageGuard", true, None::<&str>)?,
            ],
        )
    }
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

fn build_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<()> {
    let menu = create_tray_menu(&app.handle())?;

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
}

#[cfg(target_os = "linux")]
fn setup_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<bool> {
    match build_tray(app) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(not(target_os = "linux"))]
fn setup_tray(app: &mut tauri::App<tauri::Wry>) -> tauri::Result<bool> {
    build_tray(app)?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn show_main_window(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        TRAY_TOGGLE_ID => toggle_window(app),
        TRAY_PROVIDERS_ID | CTX_PROVIDERS_ID => spawn_open_provider_settings(app.clone()),
        TRAY_START_ON_LOGIN_ID | CTX_START_ON_LOGIN_ID => {
            let enabled = !cached_start_on_login_enabled(app);

            if set_start_on_login_enabled(enabled).is_err() {
                emit_native_notification("UsageGuard", START_ON_LOGIN_FAILURE_MESSAGE);
            } else {
                set_cached_start_on_login_enabled(app, enabled);
            }
        }
        TRAY_QUIT_ID | CTX_QUIT_ID => save_position_and_exit(app),
        CTX_REFRESH_ID => {
            usageguard_core::invalidate_claude_local_insights_cache();
            usageguard_core::invalidate_codex_wham_cache();

            spawn_snapshot_refresh(app.clone());
        }
        CTX_ALWAYS_ON_TOP_ID => {
            if let Some(win) = app.get_webview_window("main") {
                if let Ok(current) = win.is_always_on_top() {
                    let _ = win.set_always_on_top(!current);
                }
            }
        }
        CTX_LIGHT_MODE_ID => {
            let state = app.state::<AppState>();

            {
                let mut cfg = state.cfg.lock().expect("AppState cfg lock poisoned");

                cfg.light_mode = !cfg.light_mode;

                let _ = save_config(&cfg);
            }

            emit_widget_refresh(app);
        }
        CTX_HIDE_ID => {
            if tray_available(app) {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.hide();
                }
            }
        }
        _ => {}
    }
}

#[derive(Serialize)]
struct ConsumerStatus {
    connected: bool,
    enabled: bool,
    plan_type: Option<String>,
    label: Option<String>,
    alerts_5h_enabled: bool,
    alerts_week_enabled: bool,
    supports_usage: bool,
    supports_5h_usage: bool,
    supports_week_usage: bool,
    source_label: String,
    status_message: Option<String>,
}

fn latest_consumer_snapshot(state: &AppState, provider: &str) -> Option<UsageSnapshot> {
    state
        .snapshots
        .lock()
        .expect("AppState snapshots lock poisoned")
        .iter()
        .find(|view| {
            view.snapshot.provider == provider
                && matches!(
                    view.snapshot.source.as_str(),
                    "consumer_local" | "consumer_local_status"
                )
        })
        .map(|view| view.snapshot.clone())
}

fn snapshot_window_supported(snapshot: &UsageSnapshot, window: &'static str) -> bool {
    if snapshot.source != "consumer_local" {
        return false;
    }

    let quota_window = match window {
        "primary" => snapshot
            .consumer_quota
            .as_ref()
            .and_then(|quota| quota.primary.as_ref()),
        "secondary" => snapshot
            .consumer_quota
            .as_ref()
            .and_then(|quota| quota.secondary.as_ref()),
        _ => None,
    };

    quota_window.is_some_and(|entry| entry.available && entry.used_percent.is_some())
}

#[tauri::command]
fn get_openai_consumer_status(state: State<AppState>) -> ConsumerStatus {
    let connected = usageguard_core::has_openai_consumer_source();

    let snapshot = latest_consumer_snapshot(state.inner(), "openai");

    let supports_5h_usage = snapshot
        .as_ref()
        .is_some_and(|value| snapshot_window_supported(value, "primary"));

    let supports_week_usage = snapshot
        .as_ref()
        .is_some_and(|value| snapshot_window_supported(value, "secondary"));

    let supports_usage = supports_5h_usage || supports_week_usage;

    let plan_type = if connected {
        usageguard_core::get_openai_consumer_plan_type().filter(|s| !s.is_empty())
    } else {
        None
    };

    let cfg = state
        .cfg
        .lock()
        .expect("AppState cfg lock poisoned")
        .clone();

    ConsumerStatus {
        connected,
        enabled: true,
        plan_type,
        label: cfg.openai_consumer_label,
        alerts_5h_enabled: cfg.openai_consumer_5h_alerts_enabled,
        alerts_week_enabled: cfg.openai_consumer_week_alerts_enabled,
        supports_usage,
        supports_5h_usage,
        supports_week_usage,
        source_label: "Codex local client".to_string(),
        status_message: if !connected {
            Some("Sign in to Codex on this machine to enable local usage import.".to_string())
        } else if let Some(snapshot) = snapshot {
            snapshot.status_message.clone()
        } else if !supports_usage {
            Some("Signed in locally. Usage appears after your next Codex request.".to_string())
        } else if connected {
            None
        } else {
            None
        },
    }
}

#[tauri::command]
fn get_anthropic_consumer_status(state: State<AppState>) -> ConsumerStatus {
    let connected = usageguard_core::has_anthropic_consumer_source();

    let snapshot = latest_consumer_snapshot(state.inner(), "anthropic");

    let supports_5h_usage = snapshot
        .as_ref()
        .is_some_and(|value| snapshot_window_supported(value, "primary"));

    let supports_week_usage = snapshot
        .as_ref()
        .is_some_and(|value| snapshot_window_supported(value, "secondary"));

    let supports_usage = supports_5h_usage || supports_week_usage;

    let plan_type = if connected {
        usageguard_core::get_anthropic_consumer_plan_type().filter(|s| !s.is_empty())
    } else {
        None
    };

    let cfg = state
        .cfg
        .lock()
        .expect("AppState cfg lock poisoned")
        .clone();

    ConsumerStatus {
        connected,
        enabled: cfg.anthropic_consumer_enabled,
        plan_type,
        label: cfg.anthropic_consumer_label,
        alerts_5h_enabled: cfg.anthropic_consumer_5h_alerts_enabled,
        alerts_week_enabled: cfg.anthropic_consumer_week_alerts_enabled,
        supports_usage,
        supports_5h_usage,
        supports_week_usage,
        source_label: "Claude Code local client".to_string(),
        status_message: if !connected {
            Some("Sign in to Claude Code on this machine to enable local detection.".to_string())
        } else if let Some(snapshot) = snapshot {
            if supports_week_usage {
                Some(
                    "Showing exact Claude Code 5h and week quota from locally sourced usage data."
                        .to_string(),
                )
            } else if supports_5h_usage {
                Some(
                    "Showing exact Claude Code 5h quota from locally sourced usage data. Weekly quota is not currently available from local data."
                        .to_string(),
                )
            } else {
                snapshot.status_message.clone().or_else(|| {
                    Some("Claude Code local sign-in detected. Quota data is syncing.".to_string())
                })
            }
        } else if connected && supports_week_usage {
            Some(
                "Showing exact Claude Code 5h and week quota from locally sourced usage data."
                    .to_string(),
            )
        } else if connected && supports_5h_usage {
            Some(
                "Showing exact Claude Code 5h quota from locally sourced usage data. Weekly quota is not currently available from local data."
                    .to_string(),
            )
        } else if connected {
            Some("Claude Code local sign-in detected. Quota data is syncing.".to_string())
        } else {
            None
        },
    }
}

#[tauri::command]
fn set_consumer_label(
    window: WebviewWindow,
    provider: String,
    label: String,
    state: State<AppState>,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "set_consumer_label")?;

    let trimmed = label.trim().to_string();

    let label_opt = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };

    let mut guard = state.cfg.lock().expect("AppState cfg lock poisoned");

    match provider.as_str() {
        "openai" => guard.openai_consumer_label = label_opt,
        "anthropic" => guard.anthropic_consumer_label = label_opt,
        _ => return Err(format!("Unknown consumer provider: {provider}")),
    }

    save_config(&guard).map_err(|e| e.to_string())?;

    drop(guard);

    spawn_snapshot_refresh(app);

    Ok(())
}

#[tauri::command]
fn set_consumer_window_alerts_enabled(
    window: WebviewWindow,
    provider: String,
    window_key: String,
    enabled: bool,
    state: State<AppState>,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(
        &window,
        SETTINGS_LABEL,
        "set_consumer_window_alerts_enabled",
    )?;

    {
        let mut cfg = state.cfg.lock().expect("AppState cfg lock poisoned");

        match (provider.as_str(), window_key.as_str()) {
            ("openai", "5h") => cfg.openai_consumer_5h_alerts_enabled = enabled,
            ("openai", "week") => cfg.openai_consumer_week_alerts_enabled = enabled,
            ("anthropic", "5h") => cfg.anthropic_consumer_5h_alerts_enabled = enabled,
            ("anthropic", "week") => cfg.anthropic_consumer_week_alerts_enabled = enabled,
            _ => return Err(format!("Unknown consumer provider: {provider}")),
        }

        save_config(&cfg).map_err(|e| e.to_string())?;

        refresh_snapshot_alert_state(&state, &cfg);
    }

    emit_widget_refresh(&app);

    spawn_snapshot_refresh(app);

    Ok(())
}

#[tauri::command]
fn set_consumer_enabled(
    window: WebviewWindow,
    provider: String,
    enabled: bool,
    state: State<AppState>,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "set_consumer_enabled")?;

    {
        let mut cfg = state.cfg.lock().expect("AppState cfg lock poisoned");

        match provider.as_str() {
            "anthropic" => cfg.anthropic_consumer_enabled = enabled,
            _ => return Err(format!("Unknown consumer provider: {provider}")),
        }

        save_config(&cfg).map_err(|e| e.to_string())?;

        refresh_snapshot_alert_state(&state, &cfg);
    }

    emit_widget_refresh(&app);

    spawn_snapshot_refresh(app);

    Ok(())
}

#[tauri::command]
fn send_test_alert(
    window: WebviewWindow,
    target: TestAlertInput,
    state: State<AppState>,
    app: AppHandle,
) -> Result<String, String> {
    require_window_label(&window, SETTINGS_LABEL, "send_test_alert")?;

    let snapshot = find_snapshot_for_test_alert(state.inner(), &target).ok_or_else(|| {
        format!(
            "No loaded provider card found for '{}'. Refresh the widget and try again.",
            target.account_label
        )
    })?;

    let alert = Alert {
        level: "warning".into(),
        code: TEST_ALERT_CODE.into(),
        message: TEST_ALERT_MESSAGE.into(),
    };

    let expires_at = Instant::now() + TEST_ALERT_DURATION;

    let target_key = snapshot_key(&snapshot);

    {
        let mut manual_alerts = state
            .manual_alerts
            .lock()
            .expect("AppState manual_alerts lock poisoned");

        prune_manual_alerts(&mut manual_alerts);

        manual_alerts.insert(
            target_key.clone(),
            ManualAlert {
                alert: alert.clone(),
                expires_at,
            },
        );
    }

    {
        let mut snapshots = state
            .snapshots
            .lock()
            .expect("AppState snapshots lock poisoned");

        if let Some(snapshot_view) = snapshots
            .iter_mut()
            .find(|view| snapshot_key(&view.snapshot) == target_key)
        {
            snapshot_view
                .alerts
                .retain(|existing_alert| existing_alert.code != TEST_ALERT_CODE);

            snapshot_view.alerts.push(alert.clone());
        }
    }

    remember_emitted_alert(
        &mut state
            .notified_alerts
            .lock()
            .expect("AppState notified_alerts lock poisoned"),
        &snapshot,
        &alert,
    );

    emit_native_notification(
        "UsageGuard",
        &format!("{}: {}", snapshot.account_label, alert.message),
    );

    emit_widget_refresh(&app);

    spawn_manual_alert_expiry(app, target_key, expires_at);

    Ok(snapshot.account_label)
}

#[cfg(test)]
mod tests {

    use super::{
        alert_cycle_key, anthropic_snapshot_needs_bootstrap, apply_manual_alerts,
        clamp_popup_origin_to_area, clamp_widget_origin_to_area, collect_pending_notifications,
        compare_versions, default_widget_origin_for_area, prune_manual_alerts,
        remember_emitted_alert, snapshot_window_supported, work_area_contains_point, Alert,
        AppConfig, LogicalWorkArea, ManualAlert, SnapshotView, UsageSnapshot,
        CONSUMER_LOCAL_STATUS_SOURCE, CONSUMER_LOCAL_USAGE_PENDING_CODE,
        CONSUMER_LOCAL_WAITING_FOR_USAGE_CODE, DEFAULT_WIDGET_HEIGHT, DEFAULT_WIDGET_MARGIN_BOTTOM,
        DEFAULT_WIDGET_MARGIN_RIGHT, DEFAULT_WIDGET_WIDTH, TEST_ALERT_CODE, TEST_ALERT_MESSAGE,
    };
    #[cfg(target_os = "linux")]
    use super::{
        current_executable_path, desktop_exec_value, is_start_on_login_enabled,
        linux_autostart_file_contents, linux_autostart_path, set_start_on_login_enabled,
        XDG_CONFIG_HOME_ENV,
    };
    use chrono::{Local, Timelike};
    use std::cmp::Ordering;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    #[test]

    fn version_compare_ignores_leading_v_prefix() {
        assert_eq!(compare_versions("v0.2.0", "0.1.9"), Some(Ordering::Greater));
    }

    #[test]

    fn version_compare_pads_shorter_versions_with_zeroes() {
        assert_eq!(compare_versions("1.2", "1.2.0"), Some(Ordering::Equal));
    }

    #[test]

    fn version_compare_handles_prerelease_suffixes() {
        assert_eq!(
            compare_versions("1.2.3-beta1", "1.2.2"),
            Some(Ordering::Greater)
        );
    }

    #[test]

    fn work_area_contains_point_rejects_offscreen_corner() {
        let area = LogicalWorkArea {
            left: 0.0,
            top: 0.0,
            right: 1920.0,
            bottom: 1080.0,
        };

        assert!(work_area_contains_point(area, 1919.0, 1079.0));

        assert!(!work_area_contains_point(area, -1.0, 1079.0));
    }

    #[test]

    fn clamp_widget_origin_to_area_keeps_widget_fully_visible() {
        let area = LogicalWorkArea {
            left: 0.0,
            top: 0.0,
            right: 800.0,
            bottom: 600.0,
        };

        let (x, y) = clamp_widget_origin_to_area(area, 700.0, 550.0);

        assert_eq!(x, area.right - DEFAULT_WIDGET_WIDTH);

        assert_eq!(y, area.bottom - DEFAULT_WIDGET_HEIGHT);
    }

    #[test]

    fn default_widget_origin_anchors_to_bottom_right_margin() {
        let area = LogicalWorkArea {
            left: 0.0,
            top: 0.0,
            right: 1920.0,
            bottom: 1080.0,
        };

        let (x, y) = default_widget_origin_for_area(area);

        assert_eq!(
            x,
            area.right - DEFAULT_WIDGET_WIDTH - DEFAULT_WIDGET_MARGIN_RIGHT
        );

        assert_eq!(
            y,
            area.bottom - DEFAULT_WIDGET_HEIGHT - DEFAULT_WIDGET_MARGIN_BOTTOM
        );
    }

    #[test]

    fn clamp_popup_origin_to_area_keeps_popup_fully_visible() {
        let area = LogicalWorkArea {
            left: 0.0,
            top: 0.0,
            right: 800.0,
            bottom: 600.0,
        };

        let (x, y) = clamp_popup_origin_to_area(area, 760.0, 570.0, 360.0, 348.0);

        assert_eq!(x, 440.0);

        assert_eq!(y, 252.0);
    }

    #[test]

    fn snapshot_window_supported_requires_local_quota_data() {
        let snapshot = UsageSnapshot {
            provider: "anthropic".into(),
            account_label: "Claude Code".into(),
            spent_usd: 0.0,
            limit_usd: 0.0,
            tokens_in: 0,
            tokens_out: 0,
            inactive_hours: 0,
            source: "consumer_local".into(),
            status_code: Some("consumer_local_quota".into()),
            status_message: None,
            api_metrics: None,
            consumer_quota: Some(usageguard_core::ConsumerQuotaCard {
                primary: Some(usageguard_core::ConsumerQuotaWindow {
                    available: true,
                    used_percent: Some(62.0),
                    reset_at: Some("2026-03-19T10:00:00Z".into()),
                }),
                secondary: Some(usageguard_core::ConsumerQuotaWindow {
                    available: true,
                    used_percent: Some(74.0),
                    reset_at: Some("2026-03-23T10:00:00Z".into()),
                }),
            }),
            primary_reset_at: None,
            secondary_reset_at: None,
        };

        assert!(snapshot_window_supported(&snapshot, "primary"));

        assert!(snapshot_window_supported(&snapshot, "secondary"));
    }

    fn anthropic_status_snapshot(status_code: &str, with_quota: bool) -> UsageSnapshot {
        UsageSnapshot {
            provider: "anthropic".into(),
            account_label: "Claude Code".into(),
            spent_usd: 0.0,
            limit_usd: 0.0,
            tokens_in: 0,
            tokens_out: 0,
            inactive_hours: 0,
            source: CONSUMER_LOCAL_STATUS_SOURCE.into(),
            status_code: Some(status_code.into()),
            status_message: None,
            api_metrics: None,
            consumer_quota: if with_quota {
                Some(usageguard_core::ConsumerQuotaCard {
                    primary: Some(usageguard_core::ConsumerQuotaWindow {
                        available: true,
                        used_percent: Some(12.0),
                        reset_at: Some("2026-03-20T10:00:00Z".into()),
                    }),
                    secondary: None,
                })
            } else {
                None
            },
            primary_reset_at: None,
            secondary_reset_at: None,
        }
    }

    #[test]

    fn anthropic_waiting_status_without_quota_triggers_bootstrap() {
        let snapshot = anthropic_status_snapshot(CONSUMER_LOCAL_WAITING_FOR_USAGE_CODE, false);

        assert!(anthropic_snapshot_needs_bootstrap(&snapshot));
    }

    #[test]

    fn anthropic_pending_status_without_quota_triggers_bootstrap() {
        let snapshot = anthropic_status_snapshot(CONSUMER_LOCAL_USAGE_PENDING_CODE, false);

        assert!(anthropic_snapshot_needs_bootstrap(&snapshot));
    }

    #[test]

    fn anthropic_status_with_quota_does_not_trigger_bootstrap() {
        let snapshot = anthropic_status_snapshot(CONSUMER_LOCAL_WAITING_FOR_USAGE_CODE, true);

        assert!(!anthropic_snapshot_needs_bootstrap(&snapshot));
    }

    #[test]

    fn non_anthropic_status_does_not_trigger_bootstrap() {
        let mut snapshot = anthropic_status_snapshot(CONSUMER_LOCAL_WAITING_FOR_USAGE_CODE, false);
        snapshot.provider = "openai".into();

        assert!(!anthropic_snapshot_needs_bootstrap(&snapshot));
    }

    #[cfg(target_os = "linux")]

    fn autostart_env_lock() -> &'static std::sync::Mutex<()> {
        use std::sync::{Mutex, OnceLock};

        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();

        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(target_os = "linux")]

    fn with_autostart_test_dir(name: &str, test: impl FnOnce(std::path::PathBuf)) {
        let _guard = autostart_env_lock().lock().unwrap();

        let root = std::env::temp_dir().join(format!(
            "usageguard_autostart_{name}_{}",
            std::process::id()
        ));

        let _ = std::fs::remove_dir_all(&root);

        std::fs::create_dir_all(&root).unwrap();

        std::env::set_var(XDG_CONFIG_HOME_ENV, &root);

        test(root.join("autostart"));

        std::env::remove_var(XDG_CONFIG_HOME_ENV);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "linux")]
    #[test]

    fn linux_autostart_path_uses_xdg_config_home() {
        with_autostart_test_dir("path", |autostart_dir| {
            let path = linux_autostart_path().unwrap();

            assert_eq!(path, autostart_dir.join("com.usageguard.app.desktop"));
        });
    }

    #[cfg(target_os = "linux")]
    #[test]

    fn linux_autostart_file_has_required_fields() {
        with_autostart_test_dir("contents", |_autostart_dir| {
            let contents = linux_autostart_file_contents().unwrap();

            let exe = current_executable_path().unwrap();

            assert!(contents.starts_with("[Desktop Entry]\n"));

            assert!(contents.contains("Type=Application\n"));

            assert!(contents.contains("Version=1.0\n"));

            assert!(contents.contains("Name=UsageGuard\n"));

            assert!(contents.contains(&format!("Exec={}\n", desktop_exec_value(&exe))));

            assert!(contents.contains("Terminal=false\n"));
        });
    }

    #[cfg(target_os = "linux")]
    #[test]

    fn linux_autostart_state_detects_managed_file() {
        with_autostart_test_dir("detect", |autostart_dir| {
            std::fs::create_dir_all(&autostart_dir).unwrap();

            std::fs::write(
                autostart_dir.join("com.usageguard.app.desktop"),
                "[Desktop Entry]\nType=Application\nName=UsageGuard\n",
            )
            .unwrap();

            assert!(is_start_on_login_enabled());
        });
    }

    #[cfg(target_os = "linux")]
    #[test]

    fn linux_autostart_enable_disable_round_trip() {
        with_autostart_test_dir("round_trip", |_autostart_dir| {
            let path = linux_autostart_path().unwrap();

            assert!(!is_start_on_login_enabled());

            assert!(!path.exists());

            set_start_on_login_enabled(true).unwrap();

            assert!(path.exists());

            assert!(is_start_on_login_enabled());

            set_start_on_login_enabled(false).unwrap();

            assert!(!path.exists());

            assert!(!is_start_on_login_enabled());
        });
    }

    fn snapshot_view_with_alerts(
        primary_reset_at: Option<&str>,
        alerts: Vec<Alert>,
    ) -> SnapshotView {
        SnapshotView {
            snapshot: UsageSnapshot {
                provider: "openai".into(),
                account_label: "ChatGPT Plus".into(),
                spent_usd: 82.0,
                limit_usd: 100.0,
                tokens_in: 91,
                tokens_out: 0,
                inactive_hours: 0,
                source: "consumer_local".into(),
                status_code: None,
                status_message: None,
                api_metrics: None,
                consumer_quota: None,
                primary_reset_at: primary_reset_at.map(str::to_string),
                secondary_reset_at: Some("2026-03-14T00:00:00Z".into()),
            },
            alerts,
        }
    }

    fn warning_alert(code: &str, message: &str) -> Alert {
        Alert {
            level: "warning".into(),
            code: code.into(),
            message: message.into(),
        }
    }

    #[test]

    fn notification_state_does_not_reemit_unchanged_alerts() {
        let cfg = AppConfig::default();

        let now = Local::now();

        let mut notified = HashMap::new();

        let snapshot = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );

        let first = collect_pending_notifications(
            std::slice::from_ref(&snapshot),
            now,
            &cfg,
            &mut notified,
        );

        let second = collect_pending_notifications(
            std::slice::from_ref(&snapshot),
            now,
            &cfg,
            &mut notified,
        );

        assert_eq!(first.len(), 1);

        assert!(second.is_empty());
    }

    #[test]

    fn notification_state_stays_suppressed_after_clear_until_a_different_alert_rearms_it() {
        let cfg = AppConfig::default();

        let now = Local::now();

        let mut notified = HashMap::new();

        let active = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );

        let cleared = snapshot_view_with_alerts(Some("2026-03-10T12:00:00Z"), vec![]);
        let different = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_unused_before_reset",
                "5h quota reset soon with little usage",
            )],
        );

        assert_eq!(
            collect_pending_notifications(std::slice::from_ref(&active), now, &cfg, &mut notified)
                .len(),
            1
        );

        assert!(collect_pending_notifications(
            std::slice::from_ref(&cleared),
            now,
            &cfg,
            &mut notified
        )
        .is_empty());

        assert!(collect_pending_notifications(
            std::slice::from_ref(&active),
            now,
            &cfg,
            &mut notified
        )
        .is_empty());

        assert_eq!(
            collect_pending_notifications(
                std::slice::from_ref(&different),
                now,
                &cfg,
                &mut notified
            )
            .len(),
            1
        );

        assert_eq!(
            collect_pending_notifications(std::slice::from_ref(&active), now, &cfg, &mut notified)
                .len(),
            1
        );
    }

    #[test]

    fn notification_signature_rearms_for_new_reset_cycle() {
        let cfg = AppConfig::default();
        let now = Local::now();
        let mut notified = HashMap::new();

        let current_cycle = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );

        let next_cycle = snapshot_view_with_alerts(
            Some("2026-03-10T17:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );

        assert_ne!(
            alert_cycle_key(&current_cycle.snapshot, &current_cycle.alerts[0]),
            alert_cycle_key(&next_cycle.snapshot, &next_cycle.alerts[0])
        );

        assert_eq!(
            collect_pending_notifications(
                std::slice::from_ref(&current_cycle),
                now,
                &cfg,
                &mut notified
            )
            .len(),
            1
        );

        assert_eq!(
            collect_pending_notifications(
                std::slice::from_ref(&next_cycle),
                now,
                &cfg,
                &mut notified
            )
            .len(),
            1
        );
    }

    #[test]

    fn notification_state_only_emits_new_alerts_when_the_state_grows() {
        let cfg = AppConfig::default();
        let now = Local::now();
        let mut notified = HashMap::new();

        let base = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );
        let expanded = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![
                warning_alert("quota_5h_near_limit", "5h quota nearly used up"),
                warning_alert(
                    "quota_5h_unused_before_reset",
                    "5h quota reset soon with little usage",
                ),
            ],
        );

        assert_eq!(
            collect_pending_notifications(std::slice::from_ref(&base), now, &cfg, &mut notified)
                .len(),
            1
        );

        let added = collect_pending_notifications(
            std::slice::from_ref(&expanded),
            now,
            &cfg,
            &mut notified,
        );

        assert_eq!(added.len(), 1);
        assert!(added[0].1.contains("little usage"));
    }

    #[test]

    fn equivalent_reset_formats_share_the_same_cycle_signature() {
        let cfg = AppConfig::default();
        let now = Local::now();
        let mut notified = HashMap::new();

        let current_cycle = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );
        let equivalent_cycle = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00+00:00"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );

        assert_eq!(
            alert_cycle_key(&current_cycle.snapshot, &current_cycle.alerts[0]),
            alert_cycle_key(&equivalent_cycle.snapshot, &equivalent_cycle.alerts[0])
        );

        assert_eq!(
            collect_pending_notifications(
                std::slice::from_ref(&current_cycle),
                now,
                &cfg,
                &mut notified
            )
            .len(),
            1
        );

        assert!(collect_pending_notifications(
            std::slice::from_ref(&equivalent_cycle),
            now,
            &cfg,
            &mut notified
        )
        .is_empty());
    }

    #[test]

    fn quiet_hours_suppressed_alerts_emit_after_quiet_hours_end() {
        let mut quiet_cfg = AppConfig::default();
        let now = Local::now();
        let current_hour = now.hour() as u8;
        let mut notified = HashMap::new();

        quiet_cfg.quiet_hours.enabled = true;
        quiet_cfg.quiet_hours.start_hour = current_hour;
        quiet_cfg.quiet_hours.end_hour = (current_hour + 1) % 24;

        let snapshot = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );

        assert!(collect_pending_notifications(
            std::slice::from_ref(&snapshot),
            now,
            &quiet_cfg,
            &mut notified
        )
        .is_empty());

        let mut active_cfg = quiet_cfg.clone();
        active_cfg.quiet_hours.enabled = false;

        assert_eq!(
            collect_pending_notifications(
                std::slice::from_ref(&snapshot),
                now,
                &active_cfg,
                &mut notified
            )
            .len(),
            1
        );
    }

    #[test]

    fn manual_test_alert_memory_does_not_reemit_existing_alerts_on_refresh() {
        let cfg = AppConfig::default();
        let now = Local::now();
        let mut notified = HashMap::new();
        let snapshot = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![warning_alert(
                "quota_5h_near_limit",
                "5h quota nearly used up",
            )],
        );
        let manual_alert = Alert {
            level: "warning".into(),
            code: TEST_ALERT_CODE.into(),
            message: TEST_ALERT_MESSAGE.into(),
        };

        assert_eq!(
            collect_pending_notifications(
                std::slice::from_ref(&snapshot),
                now,
                &cfg,
                &mut notified
            )
            .len(),
            1
        );

        remember_emitted_alert(&mut notified, &snapshot.snapshot, &manual_alert);

        assert!(collect_pending_notifications(
            std::slice::from_ref(&snapshot),
            now,
            &cfg,
            &mut notified
        )
        .is_empty());
    }

    #[test]

    fn manual_test_alert_overlays_matching_snapshot() {
        let mut snapshot_views = vec![snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![],
        )];

        let mut manual_alerts = HashMap::new();

        manual_alerts.insert(
            "openai::ChatGPT Plus".into(),
            ManualAlert {
                alert: Alert {
                    level: "warning".into(),
                    code: TEST_ALERT_CODE.into(),
                    message: TEST_ALERT_MESSAGE.into(),
                },
                expires_at: Instant::now() + Duration::from_secs(10),
            },
        );

        apply_manual_alerts(&mut snapshot_views, &manual_alerts);

        assert_eq!(snapshot_views[0].alerts.len(), 1);

        assert_eq!(snapshot_views[0].alerts[0].code, TEST_ALERT_CODE);
    }

    #[test]

    fn prune_manual_alerts_drops_expired_entries() {
        let mut manual_alerts = HashMap::new();

        manual_alerts.insert(
            "expired".into(),
            ManualAlert {
                alert: Alert {
                    level: "warning".into(),
                    code: TEST_ALERT_CODE.into(),
                    message: TEST_ALERT_MESSAGE.into(),
                },
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );

        manual_alerts.insert(
            "fresh".into(),
            ManualAlert {
                alert: Alert {
                    level: "warning".into(),
                    code: TEST_ALERT_CODE.into(),
                    message: TEST_ALERT_MESSAGE.into(),
                },
                expires_at: Instant::now() + Duration::from_secs(10),
            },
        );

        prune_manual_alerts(&mut manual_alerts);

        assert!(!manual_alerts.contains_key("expired"));

        assert!(manual_alerts.contains_key("fresh"));
    }
}

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let mut cfg =
                load_config().map_err(|error| std::io::Error::other(error.to_string()))?;

            let saved_position = cfg.widget_position;

            let startup_enabled = initialize_windows_start_on_login(&mut cfg);

            app.manage(AppState {
                cfg: Mutex::new(cfg),
                notified_alerts: Mutex::new(HashMap::new()),
                snapshots: Mutex::new(Vec::new()),
                manual_alerts: Mutex::new(HashMap::new()),
                refresh: Mutex::new(RefreshState::default()),
                claude_bootstrap: Mutex::new(ClaudeBootstrapState::default()),
                tray_available: Mutex::new(false),
                start_on_login_enabled: Mutex::new(startup_enabled),
                tray_start_on_login_item: Mutex::new(None),
            });

            // Restore last widget position, or default to bottom-right of the work area.
            // widget_position stores the right-bottom corner so that resizeToFit (which
            // anchors to the right-bottom edge) restores correctly regardless of card count.
            if let Some(win) = app.get_webview_window("main") {
                restore_widget_position(&win, saved_position);
            }

            app.on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()));

            let tray_is_available = setup_tray(app)?;

            set_tray_available(&app.handle(), tray_is_available);

            #[cfg(target_os = "linux")]
            if !tray_is_available {
                show_main_window(&app.handle());
            }

            spawn_snapshot_refresh(app.handle().clone());

            spawn_release_check(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_snapshots,
            get_config,
            get_refresh_interval_secs,
            refresh_snapshots,
            open_provider_settings,
            quit,
            show_context_menu,
            set_window_rect,
            get_openai_consumer_status,
            get_anthropic_consumer_status,
            set_consumer_label,
            set_consumer_window_alerts_enabled,
            set_consumer_enabled,
            send_test_alert,
            set_refresh_interval_secs,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
