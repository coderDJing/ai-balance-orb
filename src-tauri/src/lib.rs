use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};
use tauri::{
    LogicalPosition, LogicalSize,
    menu::{Menu, MenuBuilder},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Emitter, Manager, Runtime, WebviewWindow, WindowEvent,
};
use tauri_plugin_updater::UpdaterExt;

const QUOTA_SCALE: f64 = 500_000.0;
const CONFIG_FILE: &str = "config.json";
const MAIN_WINDOW_DEFAULT_WIDTH: f64 = 280.0;
const MAIN_WINDOW_MIN_WIDTH: f64 = 220.0;
const MAIN_WINDOW_MAX_WIDTH: f64 = 520.0;
const MAIN_WINDOW_HEIGHT: f64 = 120.0;
const MAIN_WINDOW_MARGIN: f64 = 20.0;
const WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";
const WEBVIEW2_CRASH_REPORTER_FEATURE: &str = "msEdgeCrashReporter";
const WEBVIEW2_SHUTDOWN_FLAGS: [&str; 2] = ["--disable-crash-reporter", "--disable-breakpad"];

fn default_refresh_interval() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredConfig {
    endpoint_url: String,
    access_token: String,
    user_id: String,
    #[serde(default = "default_refresh_interval")]
    refresh_interval_secs: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientConfig {
    has_access_token: bool,
    access_token: Option<String>,
    endpoint_url: Option<String>,
    user_id: Option<String>,
    refresh_interval_secs: u64,
}

#[derive(Debug, Deserialize)]
struct NewApiSelfResponse {
    success: bool,
    message: Option<String>,
    data: Option<NewApiUserData>,
}

#[derive(Debug, Deserialize)]
struct NewApiUserData {
    quota: f64,
    username: Option<String>,
    group: Option<String>,
    request_count: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BalanceSnapshot {
    configured: bool,
    remaining: Option<f64>,
    username: Option<String>,
    group: Option<String>,
    request_count: Option<u64>,
    refreshed_at_ms: u128,
}

#[tauri::command]
fn load_config(app: AppHandle) -> Result<ClientConfig, String> {
    Ok(client_config(read_config(&app).ok()))
}

#[tauri::command]
async fn save_config(
    app: AppHandle,
    #[allow(non_snake_case)] endpointUrl: String,
    #[allow(non_snake_case)] accessToken: Option<String>,
    #[allow(non_snake_case)] userId: String,
    #[allow(non_snake_case)] refreshIntervalSecs: Option<u64>,
) -> Result<ClientConfig, String> {
    let endpoint_url = endpointUrl.trim().trim_end_matches('/').to_string();
    if endpoint_url.is_empty() {
        return Err("接口地址不能为空".to_string());
    }

    // 自动补全协议
    let endpoint_url = if endpoint_url.starts_with("http://") || endpoint_url.starts_with("https://") {
        endpoint_url
    } else if endpoint_url.starts_with("localhost") || endpoint_url.starts_with("127.0.0.1") {
        format!("http://{endpoint_url}")
    } else {
        format!("https://{endpoint_url}")
    };

    if !endpoint_url.starts_with("https://") && !endpoint_url.starts_with("http://localhost") && !endpoint_url.starts_with("http://127.0.0.1") {
        return Err("接口地址必须使用 HTTPS".to_string());
    }

    let user_id = userId.trim().to_string();
    if user_id.is_empty() {
        return Err("userId 不能为空".to_string());
    }

    let existing = read_config(&app).ok();
    let access_token = accessToken
        .unwrap_or_default()
        .trim()
        .to_string()
        .if_empty_then(|| {
            existing
                .as_ref()
                .map(|config| config.access_token.clone())
                .unwrap_or_default()
        });

    if access_token.is_empty() {
        return Err("Access Token 不能为空".to_string());
    }

    let refresh_interval_secs = refreshIntervalSecs
        .unwrap_or_else(|| {
            existing
                .as_ref()
                .map(|c| c.refresh_interval_secs)
                .unwrap_or(60)
        })
        .clamp(10, 3600);

    let config = StoredConfig {
        endpoint_url,
        access_token,
        user_id,
        refresh_interval_secs,
    };

    // 先保存配置
    write_config(&app, &config)?;

    // 尝试查询余额验证配置是否有效
    match verify_config(&config).await {
        Ok(_) => {
            let next_config = client_config(Some(config));
            let _ = app.emit_to("main", "config-saved", &next_config);
            Ok(next_config)
        }
        Err(err) => {
            // 验证失败，恢复原配置
            if let Some(old_config) = existing {
                let _ = write_config(&app, &old_config);
            }
            Err(format!("验证失败: {err}"))
        }
    }
}

#[tauri::command]
async fn query_balance(app: AppHandle) -> Result<BalanceSnapshot, String> {
    let config = match read_config(&app) {
        Ok(config)
            if !config.endpoint_url.is_empty()
                && !config.access_token.is_empty()
                && !config.user_id.is_empty() =>
        {
            config
        }
        _ => {
            return Ok(BalanceSnapshot {
                configured: false,
                remaining: None,
                username: None,
                group: None,
                request_count: None,
                refreshed_at_ms: now_ms(),
            });
        }
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent("ai-balance-orb/0.1")
        .build()
        .map_err(|err| format!("创建 HTTP 客户端失败: {err}"))?;

    let url = format!(
        "{}/api/user/self",
        config.endpoint_url.trim_end_matches('/')
    );

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", config.access_token))
        .header("New-Api-User", config.user_id)
        .send()
        .await
        .map_err(|err| format!("请求失败: {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取响应失败: {err}"))?;

    if !status.is_success() {
        return Err(format!("接口返回 HTTP {status}: {}", preview(&body)));
    }

    let parsed: NewApiSelfResponse =
        serde_json::from_str(&body).map_err(|err| format!("响应不是有效 JSON: {err}"))?;

    if !parsed.success {
        return Err(parsed
            .message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "查询失败".to_string()));
    }

    let data = parsed.data.ok_or_else(|| "响应缺少 data".to_string())?;
    Ok(BalanceSnapshot {
        configured: true,
        remaining: Some(data.quota / QUOTA_SCALE),
        username: data.username,
        group: data.group,
        request_count: data.request_count,
        refreshed_at_ms: now_ms(),
    })
}

#[tauri::command]
fn hide_window(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|err| err.to_string())
}

#[tauri::command]
fn show_settings_window(app: AppHandle) -> Result<(), String> {
    show_settings(&app).map_err(|err| err.to_string())
}

#[tauri::command]
fn resize_main_window(app: AppHandle, width: f64) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口不存在".to_string())?;
    let width = width.clamp(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MAX_WIDTH);

    window
        .set_min_size(Some(LogicalSize::new(
            MAIN_WINDOW_MIN_WIDTH,
            MAIN_WINDOW_HEIGHT,
        )))
        .map_err(|err| err.to_string())?;
    window
        .set_max_size(Some(LogicalSize::new(
            MAIN_WINDOW_MAX_WIDTH,
            MAIN_WINDOW_HEIGHT,
        )))
        .map_err(|err| err.to_string())?;
    window
        .set_size(LogicalSize::new(width, MAIN_WINDOW_HEIGHT))
        .map_err(|err| err.to_string())?;
    position_window_top_right(&window).map_err(|err| err.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    configure_webview2_shutdown_flags();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            install_dev_shutdown_handler(app.handle().clone());
            install_tray(app)?;
            position_main_window(app)?;
            start_fullscreen_monitor(app.handle().clone());
            spawn_auto_update_check(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_config,
            save_config,
            query_balance,
            hide_window,
            show_settings_window,
            resize_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn spawn_auto_update_check(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }

    tauri::async_runtime::spawn(async move {
        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(error) => {
                eprintln!("updater initialization failed: {error}");
                return;
            }
        };

        match updater.check().await {
            Ok(Some(update)) => {
                let version = update.version.clone();
                eprintln!("installing update {version}");
                match update.download_and_install(|_, _| {}, || {}).await {
                    Ok(()) => app.restart(),
                    Err(error) => eprintln!("update installation failed: {error}"),
                }
            }
            Ok(None) => {}
            Err(error) => eprintln!("update check failed: {error}"),
        }
    });
}

fn spawn_manual_update_check(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        emit_settings_hint(&app, "正在检查更新...");

        let updater = match app.updater() {
            Ok(updater) => updater,
            Err(error) => {
                eprintln!("updater initialization failed: {error}");
                emit_settings_hint(&app, "检查更新失败");
                return;
            }
        };

        match updater.check().await {
            Ok(Some(update)) => {
                let version = update.version.clone();
                emit_settings_hint(&app, &format!("发现新版本: {version}，正在安装"));
                match update.download_and_install(|_, _| {}, || {}).await {
                    Ok(()) => app.restart(),
                    Err(error) => {
                        eprintln!("update installation failed: {error}");
                        emit_settings_hint(&app, "更新安装失败");
                    }
                }
            }
            Ok(None) => emit_settings_hint(&app, "已是最新版本"),
            Err(error) => {
                eprintln!("update check failed: {error}");
                emit_settings_hint(&app, "检查更新失败");
            }
        }
    });
}

fn emit_settings_hint(app: &AppHandle, message: &str) {
    let _ = show_settings(app);
    let _ = app.emit_to("settings", "setup-required", message);
}

fn configure_webview2_shutdown_flags() {
    let existing = std::env::var(WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS).unwrap_or_default();
    let mut arguments = existing
        .split_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    merge_disabled_webview2_feature(&mut arguments);

    for flag in WEBVIEW2_SHUTDOWN_FLAGS {
        if !arguments.iter().any(|existing| existing == flag) {
            arguments.push(flag.to_string());
        }
    }

    std::env::set_var(WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS, arguments.join(" "));
}

fn merge_disabled_webview2_feature(arguments: &mut Vec<String>) {
    for argument in arguments.iter_mut() {
        let Some(disabled_features) = argument.strip_prefix("--disable-features=") else {
            continue;
        };

        if !disabled_features
            .split(',')
            .any(|feature| feature == WEBVIEW2_CRASH_REPORTER_FEATURE)
        {
            argument.push(',');
            argument.push_str(WEBVIEW2_CRASH_REPORTER_FEATURE);
        }
        return;
    }

    arguments.push(format!(
        "--disable-features={WEBVIEW2_CRASH_REPORTER_FEATURE}"
    ));
}

fn install_dev_shutdown_handler(app: AppHandle) {
    #[cfg(debug_assertions)]
    {
        if let Err(err) = ctrlc::set_handler(move || {
            app.exit(0);
        }) {
            eprintln!("安装开发退出处理失败: {err}");
        }
    }

    #[cfg(not(debug_assertions))]
    {
        let _ = app;
    }
}

fn config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|err| format!("读取配置目录失败: {err}"))?;
    fs::create_dir_all(&dir).map_err(|err| format!("创建配置目录失败: {err}"))?;
    Ok(dir.join(CONFIG_FILE))
}

fn read_config(app: &AppHandle) -> Result<StoredConfig, String> {
    let path = config_path(app)?;
    let text = fs::read_to_string(&path).map_err(|err| format!("读取配置失败: {err}"))?;
    serde_json::from_str(&text).map_err(|err| format!("解析配置失败: {err}"))
}

fn write_config(app: &AppHandle, config: &StoredConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text =
        serde_json::to_string_pretty(config).map_err(|err| format!("序列化配置失败: {err}"))?;
    fs::write(path, text).map_err(|err| format!("写入配置失败: {err}"))
}

async fn verify_config(config: &StoredConfig) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("ai-balance-orb/0.1")
        .build()
        .map_err(|err| format!("创建 HTTP 客户端失败: {err}"))?;

    let url = format!(
        "{}/api/user/self",
        config.endpoint_url.trim_end_matches('/')
    );

    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", config.access_token))
        .header("New-Api-User", &config.user_id)
        .send()
        .await
        .map_err(|err| format!("请求失败: {err}"))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("读取响应失败: {err}"))?;

    if !status.is_success() {
        return Err(format!("接口返回 HTTP {status}: {}", preview(&body)));
    }

    let parsed: NewApiSelfResponse =
        serde_json::from_str(&body).map_err(|err| format!("响应不是有效 JSON: {err}"))?;

    if !parsed.success {
        return Err(parsed
            .message
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| "查询失败".to_string()));
    }

    Ok(())
}

fn client_config(config: Option<StoredConfig>) -> ClientConfig {
    ClientConfig {
        has_access_token: config
            .as_ref()
            .is_some_and(|config| !config.access_token.is_empty()),
        access_token: config
            .as_ref()
            .map(|config| config.access_token.clone())
            .filter(|token| !token.is_empty()),
        endpoint_url: config
            .as_ref()
            .map(|config| config.endpoint_url.clone())
            .filter(|endpoint_url| !endpoint_url.is_empty()),
        user_id: config
            .as_ref()
            .map(|config| config.user_id.clone())
            .filter(|user_id| !user_id.is_empty()),
        refresh_interval_secs: config
            .map(|config| config.refresh_interval_secs)
            .unwrap_or(60),
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn preview(value: &str) -> String {
    const MAX: usize = 220;
    if value.len() <= MAX {
        return value.to_string();
    }

    let mut end = MAX;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}...", &value[..end])
}

fn install_tray(app: &mut App) -> tauri::Result<()> {
    let handle = app.handle();
    let menu = build_tray_menu(handle)?;

    let mut tray = TrayIconBuilder::with_id("main")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("AI Balance Orb")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "settings" => {
                let _ = show_settings(app);
            }
            "check_update" => spawn_manual_update_check(app.clone()),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle()),
            _ => {}
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }

    app.manage(tray.build(handle)?);
    Ok(())
}

fn build_tray_menu<R, M>(manager: &M) -> tauri::Result<Menu<R>>
where
    R: Runtime,
    M: Manager<R>,
{
    MenuBuilder::new(manager)
        .text("settings", "设置")
        .text("check_update", "检查更新")
        .separator()
        .text("quit", "退出")
        .build()
}

fn show_main_window(app: &AppHandle) {
    // 检查配置是否完整，未配置时只显示设置窗口并提示
    let config_ready = read_config(app).map_or(false, |config| {
        !config.endpoint_url.is_empty()
            && !config.access_token.is_empty()
            && !config.user_id.is_empty()
    });

    if !config_ready {
        let _ = app.emit_to("settings", "setup-required", "请先完成配置才能显示余额悬浮球");
        let _ = show_settings(app);
        return;
    }

    if let Some(window) = app.get_webview_window("main") {
        reveal_window(&window);
    }
}

fn reveal_window(window: &WebviewWindow) {
    // 先移除再重新设置，强制触发 SetWindowPos(HWND_TOPMOST)
    let _ = window.set_always_on_top(false);
    let _ = window.set_always_on_top(true);
    let _ = window.show();
    let _ = window.set_focus();
}

fn show_settings(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("settings") {
        window.show()?;
        window.set_focus()?;
    }

    Ok(())
}

fn position_main_window(app: &App) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        position_window_top_right(&window)?;
    } else {
        eprintln!("[position] main window not found");
    }

    Ok(())
}

fn position_window_top_right(window: &WebviewWindow) -> tauri::Result<()> {
    let window_width = logical_window_width(window);

    let monitor = match window.current_monitor()? {
        Some(monitor) => Some(monitor),
        None => window.primary_monitor()?,
    };

    if let Some(monitor) = monitor {
        let size = monitor.size();
        let position = monitor.position();
        let scale = monitor.scale_factor();
        let x = position.x as f64 / scale + size.width as f64 / scale
            - window_width
            - MAIN_WINDOW_MARGIN;
        let y = position.y as f64 / scale + MAIN_WINDOW_MARGIN;
        window.set_position(tauri::Position::Logical(LogicalPosition::new(x, y)))?;
        return Ok(());
    }

    let x = 1920.0 - window_width - MAIN_WINDOW_MARGIN;
    window.set_position(tauri::Position::Logical(LogicalPosition::new(
        x,
        MAIN_WINDOW_MARGIN,
    )))?;
    Ok(())
}

fn logical_window_width(window: &WebviewWindow) -> f64 {
    let scale = window
        .scale_factor()
        .ok()
        .filter(|scale| *scale > 0.0)
        .unwrap_or(1.0);

    window
        .inner_size()
        .map(|size| size.width as f64 / scale)
        .unwrap_or(MAIN_WINDOW_DEFAULT_WIDTH)
}

trait EmptyStringExt {
    fn if_empty_then<F>(self, fallback: F) -> String
    where
        F: FnOnce() -> String;
}

impl EmptyStringExt for String {
    fn if_empty_then<F>(self, fallback: F) -> String
    where
        F: FnOnce() -> String,
    {
        if self.is_empty() {
            fallback()
        } else {
            self
        }
    }
}

// ============ 全屏检测 ============

fn start_fullscreen_monitor(app: AppHandle) {
    let was_hidden = Arc::new(Mutex::new(false));

    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_secs(1));

            let is_fullscreen = is_fullscreen_active();
            let mut hidden = was_hidden.lock().unwrap();

            if is_fullscreen {
                if !*hidden {
                    // 全屏激活，隐藏主窗口
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.set_always_on_top(false);
                        let _ = window.hide();
                    }
                    *hidden = true;
                }
            } else if *hidden {
                // 全屏退出，恢复主窗口
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_always_on_top(false);
                    let _ = window.set_always_on_top(true);
                }
                *hidden = false;
            }
        }
    });
}

#[cfg(windows)]
fn is_fullscreen_active() -> bool {
    use windows_sys::Win32::{
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect, IsIconic},
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() || IsIconic(hwnd) != 0 {
            return false;
        }

        // 排除桌面窗口
        if is_desktop_window(hwnd) {
            return false;
        }

        let mut window_rect = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut window_rect) == 0 {
            return false;
        }

        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut monitor_info: MONITORINFO = std::mem::zeroed();
        monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;

        if GetMonitorInfoW(monitor, &mut monitor_info) == 0 {
            return false;
        }

        let monitor_rect = monitor_info.rcMonitor;
        const TOLERANCE: i32 = 2;

        window_rect.left <= monitor_rect.left + TOLERANCE
            && window_rect.top <= monitor_rect.top + TOLERANCE
            && window_rect.right >= monitor_rect.right - TOLERANCE
            && window_rect.bottom >= monitor_rect.bottom - TOLERANCE
    }
}

#[cfg(windows)]
fn is_desktop_window(hwnd: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW;

    let mut class_name = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, class_name.as_mut_ptr(), 256) };
    if len == 0 {
        return false;
    }

    let name = String::from_utf16_lossy(&class_name[..len as usize]);
    matches!(name.as_str(), "Progman" | "WorkerW")
}

#[cfg(not(windows))]
fn is_fullscreen_active() -> bool {
    false
}
