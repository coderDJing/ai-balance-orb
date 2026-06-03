use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, time::Duration};
use tauri::{
    menu::{Menu, MenuBuilder},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, Runtime, WebviewWindow, WindowEvent,
};

const QUOTA_SCALE: f64 = 500_000.0;
const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoredConfig {
    endpoint_url: String,
    access_token: String,
    user_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ClientConfig {
    has_access_token: bool,
    endpoint_url: Option<String>,
    user_id: Option<String>,
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
fn save_config(
    app: AppHandle,
    #[allow(non_snake_case)] endpointUrl: String,
    #[allow(non_snake_case)] accessToken: Option<String>,
    #[allow(non_snake_case)] userId: String,
) -> Result<ClientConfig, String> {
    let endpoint_url = endpointUrl.trim().trim_end_matches('/').to_string();
    if endpoint_url.is_empty() {
        return Err("接口地址不能为空".to_string());
    }
    if !endpoint_url.starts_with("https://") && !endpoint_url.starts_with("http://localhost") {
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
                .map(|config| config.access_token)
                .unwrap_or_default()
        });

    if access_token.is_empty() {
        return Err("Access Token 不能为空".to_string());
    }

    let config = StoredConfig {
        endpoint_url,
        access_token,
        user_id,
    };
    write_config(&app, &config)?;
    Ok(client_config(Some(config)))
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

    let response = client
        .get(&config.endpoint_url)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            install_tray(app)?;
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
            hide_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
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

fn client_config(config: Option<StoredConfig>) -> ClientConfig {
    ClientConfig {
        has_access_token: config
            .as_ref()
            .is_some_and(|config| !config.access_token.is_empty()),
        endpoint_url: config
            .as_ref()
            .map(|config| config.endpoint_url.clone())
            .filter(|endpoint_url| !endpoint_url.is_empty()),
        user_id: config
            .map(|config| config.user_id)
            .filter(|user_id| !user_id.is_empty()),
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
            "show" => show_main_window(app),
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
        .text("show", "显示余额窗")
        .separator()
        .text("quit", "退出")
        .build()
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
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
