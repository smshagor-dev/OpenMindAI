use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformCapabilities {
    pub target: &'static str,
    pub mobile: bool,
    pub desktop: bool,
    pub local_workspace: bool,
    pub full_pc_terminal: bool,
    pub managed_desktop_runtime: bool,
    pub mobile_model_runtime_ready: bool,
}

#[tauri::command]
pub fn platform_capabilities() -> PlatformCapabilities {
    let mobile = cfg!(any(target_os = "android", target_os = "ios"));
    let target = if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };

    PlatformCapabilities {
        target,
        mobile,
        desktop: !mobile,
        local_workspace: !mobile,
        full_pc_terminal: !mobile,
        managed_desktop_runtime: !mobile,
        // Android and iOS both embed the pinned llama.cpp mobile engine. The
        // platform-specific backend is selected at compile time (NDK on
        // Android, Metal on iOS), so the native app can route Chat/Thinking
        // through local inference without a desktop llama-server process.
        mobile_model_runtime_ready: mobile,
    }
}
