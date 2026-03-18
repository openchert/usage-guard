#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod icon_art;

use chrono::{DateTime, Local, Utc};
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
    clamp_refresh_interval_secs, evaluate_alerts, has_provider_account_api_key, load_config,
    provider_catalog, provider_snapshots, save_config, secure_storage_status,
    set_provider_account_api_key, should_notify_alert, verify_anthropic_oauth_access_token,
    verify_openai_oauth_access_token, verify_provider_api_key, Alert, AppConfig, ProviderAccount,
    ProviderCatalogEntry, SecureStorageStatus, UsageSnapshot, MAX_REFRESH_INTERVAL_SECS,
    MIN_REFRESH_INTERVAL_SECS,
};

#[derive(Default)]
struct RefreshState {
    in_flight: bool,
    queued: bool,
}

#[derive(Debug, Clone, Serialize)]
struct SnapshotView {
    #[serde(flatten)]
    snapshot: UsageSnapshot,
    #[serde(default)]
    alerts: Vec<Alert>,
}

/// Tracks when a specific alert was last sent and with which cycle key.
/// Used to deduplicate native notifications so they fire at most once per
/// cycle (window rotation) within a per-type cooldown period.
#[derive(Debug, Clone)]
struct NotifiedAlertEntry {
    /// Value returned by `alert_cycle_key` at the time the notification was sent.
    /// When this changes to a *new real timestamp* the alert is allowed to re-fire.
    cycle_key: String,
    /// Wall-clock time the notification was fired, for cooldown enforcement.
    fired_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
struct ManualAlert {
    alert: Alert,
    expires_at: Instant,
}

struct AppState {
    cfg: Mutex<AppConfig>,
    /// Key: `"provider::account_label::alert_code"` (no cycle component).
    /// Value: the entry from the last time this alert was actually fired.
    notified_alerts: Mutex<HashMap<String, NotifiedAlertEntry>>,
    snapshots: Mutex<Vec<SnapshotView>>,
    manual_alerts: Mutex<HashMap<String, ManualAlert>>,
    refresh: Mutex<RefreshState>,
    tray_available: Mutex<bool>,
    start_on_login_enabled: Mutex<bool>,
    tray_start_on_login_item: Mutex<Option<CheckMenuItem<tauri::Wry>>>,
}

/// After an alert notification is fired, suppress re-notification for this
/// duration *unless* the window cycle genuinely rotates (new reset timestamp).

const FIVE_HOUR_ALERT_COOLDOWN_SECS: i64 = 4 * 3600;

const WEEKLY_ALERT_COOLDOWN_SECS: i64 = 23 * 3600;

const DEFAULT_ALERT_COOLDOWN_SECS: i64 = 4 * 3600;

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
    secure_storage: SecureStorageStatus,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderAccountInput {
    id: Option<String>,
    provider: String,
    label: String,
    api_key: Option<String>,
}

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

fn apply_provider_account_save(
    cfg: &mut AppConfig,
    input: &ProviderAccountInput,
    catalog: &[ProviderCatalogEntry],
    has_api_key: impl Fn(&str) -> bool,
    validate_api_key: impl Fn(&str, &str) -> Result<(), String>,
    persist_api_key: impl Fn(&str, &str) -> Result<(), String>,
    persist_config: impl Fn(&AppConfig) -> Result<(), String>,
) -> Result<(), String> {
    let provider = normalize_required(&input.provider, "Provider")?;

    let label = normalize_required(&input.label, "Name")?;

    let api_key = normalize_optional(input.api_key.clone());

    let provider_meta = catalog
        .iter()
        .find(|entry| entry.id == provider)
        .ok_or_else(|| "Unknown provider selected".to_string())?;

    let existing_index = match input.id.as_ref() {
        Some(id) => Some(
            cfg.provider_accounts
                .iter()
                .position(|account| account.id == *id)
                .ok_or_else(|| "Provider account not found".to_string())?,
        ),
        None => None,
    };

    if let Some(index) = existing_index {
        let existing = &cfg.provider_accounts[index];

        if existing.provider != provider {
            return Err("Provider cannot be changed for an existing account".to_string());
        }
    }

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

    if api_key.is_none() && !has_api_key(&account_id) {
        return Err("API key is required".to_string());
    }

    if let Some(ref key) = api_key {
        validate_api_key(&provider, key)?;
    }

    let original_cfg = cfg.clone();

    let account = ProviderAccount {
        id: account_id.clone(),
        provider,
        label,
        endpoint: None,
    };

    if let Some(index) = existing_index {
        cfg.provider_accounts[index] = account;
    } else {
        cfg.provider_accounts.push(account);
    }

    if let Err(error) = persist_config(cfg) {
        *cfg = original_cfg;

        return Err(error);
    }

    if let Some(ref key) = api_key {
        if let Err(error) = persist_api_key(&account_id, key) {
            *cfg = original_cfg.clone();

            let _ = persist_config(&original_cfg);

            return Err(error);
        }
    }

    Ok(())
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
        secure_storage: secure_storage_status(),
    }
}

fn secure_storage_unavailable_message(status: &SecureStorageStatus) -> String {
    status.detail.clone().unwrap_or_else(|| {
        format!(
            "Secure storage backend '{}' is unavailable.",
            status.backend
        )
    })
}

fn require_secure_storage_available() -> Result<(), String> {
    let status = secure_storage_status();

    if status.available {
        Ok(())
    } else {
        Err(secure_storage_unavailable_message(&status))
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

fn refresh_notified_alert_signatures(_state: &AppState) {
    // No-op: deduplication is now time-based (see NotifiedAlertEntry / collect_pending_notifications)
    // and entries are not evicted on alert clear to prevent spurious re-notification.
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

        refresh_notified_alert_signatures(state.inner());

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

    refresh_notified_alert_signatures(state);
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
    mut config: AppConfig,
    state: State<AppState>,
    app: AppHandle,
) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "update_config")?;

    config.refresh_interval_secs = clamp_refresh_interval_secs(config.refresh_interval_secs);

    save_config(&config).map_err(|e| e.to_string())?;

    *state.cfg.lock().expect("AppState cfg lock poisoned") = config;

    emit_widget_refresh(&app);

    Ok(())
}

#[tauri::command]
fn open_provider_settings(app: AppHandle) {
    spawn_open_provider_settings(app);
}

#[tauri::command]
fn get_provider_settings(state: State<AppState>) -> ProviderSettingsPayload {
    provider_settings_payload(
        &state
            .cfg
            .lock()
            .expect("AppState cfg lock poisoned")
            .clone(),
    )
}

#[tauri::command]
fn save_provider_account(
    window: WebviewWindow,
    input: ProviderAccountInput,
    state: State<AppState>,
    app: AppHandle,
) -> Result<ProviderSettingsPayload, String> {
    require_window_label(&window, SETTINGS_LABEL, "save_provider_account")?;

    require_secure_storage_available()?;

    // Clone out because apply_provider_account_save may perform HTTP validation,
    // and we must not hold the mutex across network I/O.
    let mut cfg = state
        .cfg
        .lock()
        .expect("AppState cfg lock poisoned")
        .clone();

    let catalog = provider_catalog();

    apply_provider_account_save(
        &mut cfg,
        &input,
        &catalog,
        has_provider_account_api_key,
        |provider_id, key| {
            verify_provider_api_key(provider_id, key).map_err(|error| error.to_string())
        },
        |account_id, key| {
            set_provider_account_api_key(account_id, Some(key)).map_err(|error| error.to_string())
        },
        |updated_cfg| save_config(updated_cfg).map_err(|error| error.to_string()),
    )?;

    *state.cfg.lock().expect("AppState cfg lock poisoned") = cfg.clone();

    spawn_snapshot_refresh(app);

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

    require_secure_storage_available()?;

    let cfg = {
        let mut guard = state.cfg.lock().expect("AppState cfg lock poisoned");

        let before = guard.provider_accounts.len();

        guard.provider_accounts.retain(|account| account.id != id);

        if guard.provider_accounts.len() == before {
            return Err("Provider account not found".to_string());
        }

        save_config(&guard).map_err(|e| e.to_string())?;

        let _ = set_provider_account_api_key(&id, None);

        guard.clone()
    };

    spawn_snapshot_refresh(app);

    Ok(provider_settings_payload(&cfg))
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
    notified_alerts: &mut HashMap<String, NotifiedAlertEntry>,
) {
    for (title, body) in collect_pending_notifications(snapshots, now, cfg, notified_alerts) {
        emit_native_notification(&title, &body);
    }
}

fn collect_pending_notifications(
    snapshots: &[SnapshotView],
    now: chrono::DateTime<Local>,
    cfg: &AppConfig,
    notified_alerts: &mut HashMap<String, NotifiedAlertEntry>,
) -> Vec<(String, String)> {
    let now_utc = now.with_timezone(&Utc);

    let mut pending = Vec::new();

    for snapshot_view in snapshots {
        if snapshot_view.snapshot.source == "demo" {
            continue;
        }

        for alert in &snapshot_view.alerts {
            if !should_notify_alert(alert, now, cfg) {
                continue;
            }

            let base_key = alert_base_key(&snapshot_view.snapshot, alert);

            let cycle_key = alert_cycle_key(&snapshot_view.snapshot, alert);

            let cooldown = alert_cooldown_secs(&alert.code);

            let suppressed = match notified_alerts.get(&base_key) {
                None => false,
                Some(prev) => {
                    // Suppress if we are in the same window cycle AND within the cooldown.
                    // Two cycle keys are considered the "same cycle" when either key is a
                    // stable fallback (meaning reset_at is unknown) — this prevents a
                    // None→Some(timestamp) transition from being mistaken for a new cycle.
                    let is_stable = |k: &str| k.ends_with("-stable") || k == "stable";

                    let same_cycle = prev.cycle_key == cycle_key
                        || is_stable(&prev.cycle_key)
                        || is_stable(&cycle_key);

                    same_cycle
                        && now_utc.signed_duration_since(prev.fired_at)
                            < chrono::Duration::seconds(cooldown)
                }
            };

            if suppressed {
                continue;
            }

            notified_alerts.insert(
                base_key,
                NotifiedAlertEntry {
                    cycle_key,
                    fired_at: now_utc,
                },
            );

            pending.push((
                "UsageGuard".to_string(),
                format!(
                    "{}: {}",
                    snapshot_view.snapshot.account_label, alert.message
                ),
            ));
        }
    }

    pending
}

/// Stable map key for an alert: identifies the alert type without a cycle component.
/// The cycle is tracked separately inside `NotifiedAlertEntry`.

fn alert_base_key(snapshot: &UsageSnapshot, alert: &Alert) -> String {
    format!(
        "{}::{}::{}",
        snapshot.provider, snapshot.account_label, alert.code
    )
}

/// Returns the "cycle discriminator" for an alert — a value that changes when the
/// underlying quota window genuinely resets. Used to allow re-notification after
/// a window rotation even if the cooldown has not yet elapsed.

fn alert_cycle_key(snapshot: &UsageSnapshot, alert: &Alert) -> String {
    match alert.code.as_str() {
        "quota_5h_exhausted" | "quota_5h_near_limit" | "quota_5h_unused_before_reset" => snapshot
            .primary_reset_at
            .clone()
            .unwrap_or_else(|| "5h-stable".to_string()),
        "quota_week_exhausted" | "quota_week_near_limit" | "quota_week_unused_before_reset" => {
            snapshot
                .secondary_reset_at
                .clone()
                .unwrap_or_else(|| "week-stable".to_string())
        }
        _ => "stable".to_string(),
    }
}

/// Per-alert-type cooldown: minimum seconds between repeated notifications
/// for the same alert while the window cycle has not changed.

fn alert_cooldown_secs(code: &str) -> i64 {
    match code {
        "quota_5h_exhausted" | "quota_5h_near_limit" | "quota_5h_unused_before_reset" => {
            FIVE_HOUR_ALERT_COOLDOWN_SECS
        }
        "quota_week_exhausted" | "quota_week_near_limit" | "quota_week_unused_before_reset" => {
            WEEKLY_ALERT_COOLDOWN_SECS
        }
        _ => DEFAULT_ALERT_COOLDOWN_SECS,
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
                eprintln!("release check failed: {error}");

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

                if let Err(error) = save_config(&cfg) {
                    eprintln!("failed to persist release notification state: {error}");
                }

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
                    "Manage Providers...",
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
                        "Manage Providers...",
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
                        "Manage Providers...",
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
                    "Manage Providers...",
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
                    "Manage Providers...",
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
                    "Manage Providers...",
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
        Err(error) => {
            eprintln!("failed to create tray icon; continuing without tray: {error}");
            Ok(false)
        }
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

            if let Err(error) = set_start_on_login_enabled(enabled) {
                eprintln!("failed to update {START_ON_LOGIN_LABEL}: {error}");

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

            if let Some(win) = app.get_webview_window(SETTINGS_LABEL) {
                let _ = win.emit(REFRESH_EVENT, ());
            }
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

// --- OAuth PKCE helpers ---
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum OAuthTokenEncoding {
    Form,
    Json,
}

#[allow(dead_code)]
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

#[allow(dead_code)]
const OPENAI_OAUTH_AUTH_EXTRA_PARAMS: [(&str, &str); 2] = [
    ("id_token_add_organizations", "true"),
    ("codex_cli_simplified_flow", "true"),
];

#[allow(dead_code)]
const ANTHROPIC_OAUTH_AUTH_EXTRA_PARAMS: [(&str, &str); 0] = [];

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
fn pkce_verifier() -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use rand::RngCore;

    let mut bytes = [0u8; 32];

    rand::thread_rng().fill_bytes(&mut bytes);

    URL_SAFE_NO_PAD.encode(bytes)
}

#[allow(dead_code)]
fn pkce_challenge(verifier: &str) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use sha2::{Digest, Sha256};

    let hash = Sha256::digest(verifier.as_bytes());

    URL_SAFE_NO_PAD.encode(hash)
}

#[allow(dead_code)]
fn oauth_redirect_uri(provider: OAuthProviderConfig) -> String {
    format!(
        "http://{}:{}{}",
        provider.callback_host, provider.callback_port, provider.callback_path
    )
}

#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
fn oauth_response_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|entry| entry.as_str())
            .map(str::to_string)
    })
}

#[allow(dead_code)]
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
#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
#[tauri::command]
async fn connect_openai_oauth(window: WebviewWindow, app: AppHandle) -> Result<String, String> {
    require_window_label(&window, SETTINGS_LABEL, "connect_openai_oauth")?;

    require_secure_storage_available()?;

    let (_, access, refresh, expires_at) = authorize_with_oauth(OPENAI_OAUTH_PROVIDER).await?;

    let verified = {
        let access = access.clone();

        tauri::async_runtime::spawn_blocking(move || verify_openai_oauth_access_token(&access, ""))
            .await
            .map_err(|e| e.to_string())?
            .map_err(|e| e.to_string())?
    };

    usageguard_core::set_openai_oauth_tokens(
        &access,
        expires_at,
        &refresh,
        &verified.account_id,
        &verified.plan_type,
    )
    .map_err(|e| e.to_string())?;

    spawn_snapshot_refresh(app);

    Ok(verified.plan_type)
}

#[allow(dead_code)]
#[tauri::command]
async fn connect_anthropic_oauth(window: WebviewWindow, app: AppHandle) -> Result<String, String> {
    require_window_label(&window, SETTINGS_LABEL, "connect_anthropic_oauth")?;

    require_secure_storage_available()?;

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

    let verified = {
        let access = access.clone();

        let subscription_type = subscription_type.clone();

        let rate_limit_tier = rate_limit_tier.clone();

        tauri::async_runtime::spawn_blocking(move || {
            verify_anthropic_oauth_access_token(&access, &subscription_type, &rate_limit_tier)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?
    };

    usageguard_core::set_anthropic_oauth_tokens(
        &access,
        expires_at,
        &refresh,
        &verified.subscription_type,
        &verified.rate_limit_tier,
    )
    .map_err(|e| e.to_string())?;

    spawn_snapshot_refresh(app);

    Ok(verified.plan_type)
}

#[allow(dead_code)]
#[tauri::command]
fn disconnect_openai_oauth(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "disconnect_openai_oauth")?;

    require_secure_storage_available()?;

    usageguard_core::clear_openai_oauth_tokens();

    spawn_snapshot_refresh(app);

    Ok(())
}

#[allow(dead_code)]
#[tauri::command]
fn disconnect_anthropic_oauth(window: WebviewWindow, app: AppHandle) -> Result<(), String> {
    require_window_label(&window, SETTINGS_LABEL, "disconnect_anthropic_oauth")?;

    require_secure_storage_available()?;

    usageguard_core::clear_anthropic_oauth_tokens();

    spawn_snapshot_refresh(app);

    Ok(())
}

#[derive(Serialize)]
struct ConsumerStatus {
    connected: bool,
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

#[tauri::command]
fn get_openai_consumer_status(state: State<AppState>) -> ConsumerStatus {
    let connected = usageguard_core::has_openai_consumer_source();

    let supports_usage = usageguard_core::has_openai_consumer_usage();

    let supports_5h_usage = supports_usage;

    let supports_week_usage = supports_usage;

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
        plan_type,
        label: cfg.openai_oauth_label,
        alerts_5h_enabled: cfg.openai_oauth_5h_alerts_enabled,
        alerts_week_enabled: cfg.openai_oauth_week_alerts_enabled,
        supports_usage,
        supports_5h_usage,
        supports_week_usage,
        source_label: "Codex local client".to_string(),
        status_message: if connected && !supports_usage {
            Some("Signed in locally. Usage appears after your next Codex request.".to_string())
        } else if connected {
            None
        } else {
            Some("Sign in to Codex on this machine to enable local usage import.".to_string())
        },
    }
}

#[tauri::command]
fn get_anthropic_consumer_status(state: State<AppState>) -> ConsumerStatus {
    let connected = usageguard_core::has_anthropic_consumer_source();

    let supports_5h_usage = usageguard_core::has_anthropic_consumer_5h_usage();

    let supports_week_usage = usageguard_core::has_anthropic_consumer_week_usage();

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
        plan_type,
        label: cfg.anthropic_oauth_label,
        alerts_5h_enabled: cfg.anthropic_oauth_5h_alerts_enabled,
        alerts_week_enabled: cfg.anthropic_oauth_week_alerts_enabled,
        supports_usage,
        supports_5h_usage,
        supports_week_usage,
        source_label: "Claude Code local client".to_string(),
        status_message: if connected && supports_5h_usage {
            Some(
                "Showing exact Claude Code 5h quota from local CLI insights. Weekly quota percentage is not exposed by the local client."
                    .to_string(),
            )
        } else if connected {
            Some(
                "Claude Code local sign-in detected. Exact 5h quota is currently unavailable, and weekly quota percentage is not exposed by the local client."
                    .to_string(),
            )
        } else {
            Some("Sign in to Claude Code on this machine to enable local detection.".to_string())
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
        "openai" => guard.openai_oauth_label = label_opt,
        "anthropic" => guard.anthropic_oauth_label = label_opt,
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
            ("openai", "5h") => cfg.openai_oauth_5h_alerts_enabled = enabled,
            ("openai", "week") => cfg.openai_oauth_week_alerts_enabled = enabled,
            ("anthropic", "5h") => cfg.anthropic_oauth_5h_alerts_enabled = enabled,
            ("anthropic", "week") => cfg.anthropic_oauth_week_alerts_enabled = enabled,
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

    state
        .notified_alerts
        .lock()
        .expect("AppState notified_alerts lock poisoned")
        .insert(
            alert_base_key(&snapshot, &alert),
            NotifiedAlertEntry {
                cycle_key: alert_cycle_key(&snapshot, &alert),
                fired_at: Utc::now(),
            },
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
        alert_signature, apply_manual_alerts, apply_provider_account_save,
        clamp_widget_origin_to_area, collect_pending_notifications, compare_versions,
        default_widget_origin_for_area, parse_callback_code, prune_manual_alerts,
        work_area_contains_point, Alert, AppConfig, LogicalWorkArea, ManualAlert, ProviderAccount,
        ProviderAccountInput, ProviderCatalogEntry, SnapshotView, UsageSnapshot,
        ANTHROPIC_OAUTH_PROVIDER, DEFAULT_WIDGET_HEIGHT, DEFAULT_WIDGET_MARGIN_BOTTOM,
        DEFAULT_WIDGET_MARGIN_RIGHT, DEFAULT_WIDGET_WIDTH, OPENAI_OAUTH_PROVIDER, TEST_ALERT_CODE,
        TEST_ALERT_MESSAGE,
    };
    #[cfg(target_os = "linux")]
    use super::{
        current_executable_path, desktop_exec_value, is_start_on_login_enabled,
        linux_autostart_file_contents, linux_autostart_path, set_start_on_login_enabled,
        XDG_CONFIG_HOME_ENV,
    };
    use chrono::Local;
    use std::cell::Cell;
    use std::cmp::Ordering;
    use std::collections::{HashMap, HashSet};
    use std::time::{Duration, Instant};

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

    fn provider_catalog_fixture() -> Vec<ProviderCatalogEntry> {
        vec![
            ProviderCatalogEntry {
                id: "openai".into(),
                label: "OpenAI".into(),
            },
            ProviderCatalogEntry {
                id: "anthropic".into(),
                label: "Anthropic".into(),
            },
        ]
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

    #[test]

    fn provider_change_on_edit_is_rejected_server_side() {
        let mut cfg = AppConfig::default();

        cfg.provider_accounts.push(ProviderAccount {
            id: "acct_openai_work".into(),
            provider: "openai".into(),
            label: "Work".into(),
            endpoint: None,
        });

        let input = ProviderAccountInput {
            id: Some("acct_openai_work".into()),
            provider: "anthropic".into(),
            label: "Work".into(),
            api_key: None,
        };

        let error = apply_provider_account_save(
            &mut cfg,
            &input,
            &provider_catalog_fixture(),
            |_| true,
            |_, _| Ok(()),
            |_, _| Ok(()),
            |_| Ok(()),
        )
        .unwrap_err();

        assert!(error.contains("Provider cannot be changed"));
    }

    #[test]

    fn provider_account_input_rejects_removed_individual_fields() {
        let error = serde_json::from_value::<ProviderAccountInput>(serde_json::json!({
            "provider": "openai",
            "label": "Work",
            "apiKey": "sk-test",
            "accessMode": "individual",
            "usageLogPath": "C:\\logs\\usage.ndjson"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]

    fn label_only_edit_keeps_existing_key_without_validation() {
        let mut cfg = AppConfig::default();

        cfg.provider_accounts.push(ProviderAccount {
            id: "acct_openai_work".into(),
            provider: "openai".into(),
            label: "Work".into(),
            endpoint: None,
        });

        let validation_called = Cell::new(false);

        let persist_key_called = Cell::new(false);

        let input = ProviderAccountInput {
            id: Some("acct_openai_work".into()),
            provider: "openai".into(),
            label: "Renamed".into(),
            api_key: None,
        };

        apply_provider_account_save(
            &mut cfg,
            &input,
            &provider_catalog_fixture(),
            |_| true,
            |_, _| {
                validation_called.set(true);

                Ok(())
            },
            |_, _| {
                persist_key_called.set(true);

                Ok(())
            },
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(cfg.provider_accounts[0].label, "Renamed");

        assert!(!validation_called.get());

        assert!(!persist_key_called.get());
    }

    #[test]

    fn invalid_new_api_key_rejects_without_persisting() {
        let mut cfg = AppConfig::default();

        let persisted_key = Cell::new(false);

        let persisted_config = Cell::new(false);

        let input = ProviderAccountInput {
            id: None,
            provider: "openai".into(),
            label: "Work".into(),
            api_key: Some("bad-key".into()),
        };

        let error = apply_provider_account_save(
            &mut cfg,
            &input,
            &provider_catalog_fixture(),
            |_| false,
            |_, _| Err("OpenAI API key is invalid. Nothing was saved.".into()),
            |_, _| {
                persisted_key.set(true);

                Ok(())
            },
            |_| {
                persisted_config.set(true);

                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("invalid"));

        assert!(cfg.provider_accounts.is_empty());

        assert!(!persisted_key.get());

        assert!(!persisted_config.get());
    }

    #[test]

    fn key_replacement_failure_keeps_old_key_and_config() {
        let mut cfg = AppConfig::default();

        cfg.provider_accounts.push(ProviderAccount {
            id: "acct_openai_work".into(),
            provider: "openai".into(),
            label: "Work".into(),
            endpoint: None,
        });

        let persisted_key = Cell::new(false);

        let persisted_config = Cell::new(false);

        let input = ProviderAccountInput {
            id: Some("acct_openai_work".into()),
            provider: "openai".into(),
            label: "Work".into(),
            api_key: Some("replacement".into()),
        };

        let error = apply_provider_account_save(
            &mut cfg,
            &input,
            &provider_catalog_fixture(),
            |_| true,
            |_, _| Err("OpenAI API key is invalid. Nothing was saved.".into()),
            |_, _| {
                persisted_key.set(true);

                Ok(())
            },
            |_| {
                persisted_config.set(true);

                Ok(())
            },
        )
        .unwrap_err();

        assert!(error.contains("invalid"));

        assert_eq!(cfg.provider_accounts[0].label, "Work");

        assert!(!persisted_key.get());

        assert!(!persisted_config.get());
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
                source: "oauth".into(),
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

    #[test]

    fn notification_state_does_not_reemit_unchanged_alerts() {
        let cfg = AppConfig::default();

        let now = Local::now();

        let mut notified = HashMap::new();

        let snapshot = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![Alert {
                level: "warning".into(),
                code: "quota_5h_near_limit".into(),
                message: "5h quota nearly used up".into(),
            }],
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

    fn notification_state_suppressed_during_cooldown_even_after_clear() {
        // After firing once, alert is suppressed for cooldown window even if it clears and returns.
        // This prevents the alert from re-firing on transient fluctuations.
        let cfg = AppConfig::default();

        let now = Local::now();

        let mut notified = HashMap::new();

        let active = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![Alert {
                level: "warning".into(),
                code: "quota_5h_near_limit".into(),
                message: "5h quota nearly used up".into(),
            }],
        );

        let cleared = snapshot_view_with_alerts(Some("2026-03-10T12:00:00Z"), vec![]);

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

        // Same `now` = well within 4h cooldown — should NOT re-fire.
        assert!(collect_pending_notifications(
            std::slice::from_ref(&active),
            now,
            &cfg,
            &mut notified
        )
        .is_empty());
    }

    #[test]

    fn notification_signature_rearms_for_new_reset_cycle() {
        let current_cycle = snapshot_view_with_alerts(
            Some("2026-03-10T12:00:00Z"),
            vec![Alert {
                level: "warning".into(),
                code: "quota_5h_near_limit".into(),
                message: "5h quota nearly used up".into(),
            }],
        );

        let next_cycle = snapshot_view_with_alerts(
            Some("2026-03-10T17:00:00Z"),
            vec![Alert {
                level: "warning".into(),
                code: "quota_5h_near_limit".into(),
                message: "5h quota nearly used up".into(),
            }],
        );

        let current_signature = alert_signature(&current_cycle.snapshot, &current_cycle.alerts[0]);

        let next_signature = alert_signature(&next_cycle.snapshot, &next_cycle.alerts[0]);

        assert_ne!(current_signature, next_signature);
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
            let cfg = load_config().map_err(|error| std::io::Error::other(error.to_string()))?;

            let saved_position = cfg.widget_position;

            let startup_enabled = is_start_on_login_enabled();

            app.manage(AppState {
                cfg: Mutex::new(cfg),
                notified_alerts: Mutex::new(HashMap::new()),
                snapshots: Mutex::new(Vec::new()),
                manual_alerts: Mutex::new(HashMap::new()),
                refresh: Mutex::new(RefreshState::default()),
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
            get_provider_settings,
            open_provider_settings,
            save_provider_account,
            delete_provider_account,
            update_config,
            quit,
            show_context_menu,
            set_window_rect,
            get_openai_consumer_status,
            get_anthropic_consumer_status,
            set_consumer_label,
            set_consumer_window_alerts_enabled,
            send_test_alert,
            set_refresh_interval_secs,
        ])
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
