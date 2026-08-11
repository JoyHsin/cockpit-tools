//! Safe, narrow integration with an upstream-managed Codex Router installation.
//!
//! Router remains responsible for provider credentials and its own generated
//! configuration. Cockpit only reads its non-secret installation manifest and
//! delegates start/stop/restart to Router's documented service entry point.

use crate::modules::codex_account;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const ROUTER_STATE_DIR: &str = "codex-router";
const INSTALL_MANIFEST_FILE: &str = "install-manifest.json";
const SIGNED_PROVIDER_MODE_FILE: &str = "signed-provider-mode.json";
const DEFAULT_ROUTER_PORT: u16 = 4102;
const ROUTER_HEALTH_CONNECT_TIMEOUT: Duration = Duration::from_millis(750);
const ROUTER_REPOSITORY_URL: &str = "https://github.com/duolahypercho/codex-router.git";
const ROUTER_MANAGED_CHECKOUT_DIR: &str = "codex-router-source";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRouterStatus {
    pub installed: bool,
    pub configured: bool,
    pub running: bool,
    pub version: Option<String>,
    pub enabled_provider_count: usize,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRouterProvider {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub configured: bool,
    pub cli_installed: Option<bool>,
    pub cli_runnable: Option<bool>,
    pub action: String,
    pub visible: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRouterDoctorCheck {
    pub status: String,
    pub name: String,
    pub detail: String,
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexRouterDoctorReport {
    pub ok: bool,
    pub checks: Vec<CodexRouterDoctorCheck>,
}

#[derive(Debug, Deserialize)]
struct RouterInstallManifest {
    version: u32,
    current: Option<RouterInstallManifestCurrent>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterInstallManifestCurrent {
    source_root: Option<String>,
    package_version: Option<String>,
    providers: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterSignedProviderMode {
    managed_base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterProviderSnapshot {
    providers: Vec<RouterProviderSnapshotEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterProviderSnapshotEntry {
    id: String,
    display_name: String,
    kind: String,
    configured: bool,
    cli_installed: Option<bool>,
    cli_runnable: Option<bool>,
    action: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterDoctorSnapshot {
    ok: bool,
    checks: Vec<RouterDoctorSnapshotCheck>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterDoctorSnapshotCheck {
    status: String,
    name: String,
    detail: String,
    fix: Option<String>,
}

fn router_state_dir(codex_home: &Path) -> PathBuf {
    codex_home.join(ROUTER_STATE_DIR)
}

fn managed_checkout_dir() -> Result<PathBuf, String> {
    Ok(crate::modules::account::get_data_dir()?.join(ROUTER_MANAGED_CHECKOUT_DIR))
}

fn read_install_manifest(state_dir: &Path) -> Option<RouterInstallManifest> {
    let content = fs::read_to_string(state_dir.join(INSTALL_MANIFEST_FILE)).ok()?;
    let manifest = serde_json::from_str::<RouterInstallManifest>(&content).ok()?;
    (manifest.version == 1).then_some(manifest)
}

fn enabled_provider_count(current: Option<&RouterInstallManifestCurrent>) -> usize {
    current
        .and_then(|entry| entry.providers.as_ref())
        .map(|providers| match providers {
            serde_json::Value::Array(values) => values.len(),
            serde_json::Value::Object(values) => values.len(),
            _ => 0,
        })
        .unwrap_or(0)
}

fn router_source_root(manifest: &RouterInstallManifest) -> Option<PathBuf> {
    let raw = manifest.current.as_ref()?.source_root.as_deref()?.trim();
    if raw.is_empty() {
        return None;
    }
    let source_root = PathBuf::from(raw).canonicalize().ok()?;
    (source_root.join("src").join("service.mjs").is_file()).then_some(source_root)
}

fn is_router_source_root(path: &Path) -> bool {
    path.join("src").join("service.mjs").is_file()
        && path.join("src").join("control.mjs").is_file()
}

fn existing_router_source_root(manifest: Option<&RouterInstallManifest>) -> Option<PathBuf> {
    manifest
        .and_then(router_source_root)
        .or_else(|| managed_checkout_dir().ok())
        .and_then(|path| path.canonicalize().ok())
        .filter(|path| is_router_source_root(path))
}

fn router_port_from_state(state_dir: &Path) -> u16 {
    let content = match fs::read_to_string(state_dir.join(SIGNED_PROVIDER_MODE_FILE)) {
        Ok(content) => content,
        Err(_) => return DEFAULT_ROUTER_PORT,
    };
    let state = match serde_json::from_str::<RouterSignedProviderMode>(&content) {
        Ok(state) => state,
        Err(_) => return DEFAULT_ROUTER_PORT,
    };
    let Some(url) = state.managed_base_url else {
        return DEFAULT_ROUTER_PORT;
    };
    let Ok(parsed) = reqwest::Url::parse(&url) else {
        return DEFAULT_ROUTER_PORT;
    };
    let is_loopback = match parsed.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("localhost.")
        }
        None => false,
    };
    if !is_loopback {
        return DEFAULT_ROUTER_PORT;
    }
    parsed.port_or_known_default().unwrap_or(DEFAULT_ROUTER_PORT)
}

fn router_is_listening(state_dir: &Path) -> bool {
    let address = SocketAddr::from(([127, 0, 0, 1], router_port_from_state(state_dir)));
    let Ok(mut stream) = TcpStream::connect_timeout(&address, ROUTER_HEALTH_CONNECT_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(ROUTER_HEALTH_CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(ROUTER_HEALTH_CONNECT_TIMEOUT));
    if stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .is_err()
    {
        return false;
    }

    let mut response = Vec::with_capacity(512);
    stream.read_to_end(&mut response).is_ok()
        && response.starts_with(b"HTTP/1.1 200")
        && response
            .windows(b"\"service\":\"codex-router\"".len())
            .any(|window| window == b"\"service\":\"codex-router\"")
}

fn node_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("CODEX_ROUTER_NODE_BIN") {
        let candidate = PathBuf::from(path);
        if candidate.is_file() {
            return candidate;
        }
    }

    #[cfg(target_os = "macos")]
    for candidate in [
        "/opt/homebrew/opt/node@24/bin/node",
        "/usr/local/opt/node@24/bin/node",
    ] {
        let candidate = PathBuf::from(candidate);
        if candidate.is_file() {
            return candidate;
        }
    }

    #[cfg(target_os = "windows")]
    {
        return PathBuf::from("node.exe");
    }

    #[cfg(not(target_os = "windows"))]
    PathBuf::from("node")
}

fn router_command_environment(command: &mut Command, codex_home: &Path) {
    command.env("CODEX_HOME", codex_home);
    command.env("CODEX_ROUTER_NODE_BIN", node_binary());
    let node_path = node_binary();
    let Some(node_dir) = node_path.parent() else {
        return;
    };
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut paths = vec![node_dir.to_path_buf()];
    paths.extend(std::env::split_paths(&existing));
    if let Ok(path) = std::env::join_paths(paths) {
        command.env("PATH", path);
    }
}

fn run_node_router_command(
    source_root: &Path,
    codex_home: &Path,
    script: &str,
    args: &[&str],
) -> Result<std::process::Output, String> {
    let mut command = Command::new(node_binary());
    command
        .arg(source_root.join("src").join(script))
        .args(args)
        .current_dir(source_root);
    router_command_environment(&mut command, codex_home);
    command
        .output()
        .map_err(|_| "无法启动 Codex Router 控制命令；请检查 Node.js 运行时".to_string())
}

fn run_router_shell_command(
    source_root: &Path,
    codex_home: &Path,
    script: &str,
) -> Result<std::process::Output, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = (source_root, codex_home, script);
        return Err("Windows 下请先使用 Codex Router 官方安装器完成配置".to_string());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut command = Command::new("/bin/sh");
        command.arg(source_root.join("bin").join(script));
        command.current_dir(source_root);
        router_command_environment(&mut command, codex_home);
        return command
            .output()
            .map_err(|_| "无法启动 Codex Router 脚本；请检查系统 Shell 与 Node.js".to_string());
    }
}

fn ensure_router_source_for_install() -> Result<PathBuf, String> {
    let managed_dir = managed_checkout_dir()?;
    if is_router_source_root(&managed_dir) {
        return Ok(managed_dir);
    }
    if managed_dir.exists() {
        return Err("Cockpit 管理的 Codex Router 源码目录不完整；请删除后重试安装".to_string());
    }
    if let Some(parent) = managed_dir.parent() {
        fs::create_dir_all(parent).map_err(|_| "无法创建 Codex Router 安装目录".to_string())?;
    }
    let output = Command::new("git")
        .args(["clone", "--depth", "1", ROUTER_REPOSITORY_URL])
        .arg(&managed_dir)
        .output()
        .map_err(|_| "无法启动 git；请安装 Git 后重试".to_string())?;
    if !output.status.success() || !is_router_source_root(&managed_dir) {
        return Err("下载 Codex Router 源码失败；请检查网络和 Git 配置".to_string());
    }
    Ok(managed_dir)
}

fn require_router_source() -> Result<(PathBuf, PathBuf, RouterInstallManifest), String> {
    let codex_home = codex_account::get_codex_home();
    let state_dir = router_state_dir(&codex_home);
    let manifest = read_install_manifest(&state_dir)
        .ok_or("未检测到受支持的 Codex Router 安装记录")?;
    let source_root = existing_router_source_root(Some(&manifest))
        .ok_or("Codex Router 安装目录不可用；请使用 Router 上游工具修复安装")?;
    Ok((codex_home, source_root, manifest))
}

fn router_action_succeeded(output: &std::process::Output, action: &str) -> Result<(), String> {
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Codex Router {}失败（退出码 {}）",
            action,
            output.status.code().unwrap_or(-1)
        ))
    }
}

fn validate_provider_id(provider_id: &str) -> Result<&str, String> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty()
        || !provider_id
            .chars()
            .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-')
    {
        return Err("Codex Router Provider 标识不合法".to_string());
    }
    Ok(provider_id)
}

fn status_from_manifest(
    codex_home: &Path,
    state_dir: &Path,
    manifest: Option<&RouterInstallManifest>,
    last_error: Option<String>,
) -> CodexRouterStatus {
    let current = manifest.and_then(|entry| entry.current.as_ref());
    // A managed checkout is only a source candidate. Treat Router as installed
    // after its upstream installer has written the manifest, so a failed first
    // installation still presents a retry action in Cockpit.
    let installed = manifest.is_some() && existing_router_source_root(manifest).is_some();
    CodexRouterStatus {
        installed,
        configured: codex_account::is_codex_router_signed_routing_active(codex_home),
        running: installed && router_is_listening(state_dir),
        version: current.and_then(|entry| entry.package_version.clone()),
        enabled_provider_count: enabled_provider_count(current),
        last_error,
    }
}

pub fn get_status() -> CodexRouterStatus {
    let codex_home = codex_account::get_codex_home();
    let state_dir = router_state_dir(&codex_home);
    let manifest = read_install_manifest(&state_dir);
    status_from_manifest(&codex_home, &state_dir, manifest.as_ref(), None)
}

/// Delegates a lifecycle action to Router without passing or collecting any
/// Router provider credentials. Only Router's documented service actions are
/// accepted here.
pub fn control_service(action: &str) -> Result<CodexRouterStatus, String> {
    if !matches!(action, "start" | "stop" | "restart") {
        return Err("不支持的 Codex Router 服务操作".to_string());
    }

    let (codex_home, source_root, manifest) = require_router_source()?;
    let output = run_node_router_command(&source_root, &codex_home, "service.mjs", &[action])?;
    router_action_succeeded(&output, "服务操作")?;
    let state_dir = router_state_dir(&codex_home);

    Ok(status_from_manifest(
        &codex_home,
        &state_dir,
        Some(&manifest),
        None,
    ))
}

pub fn install() -> Result<CodexRouterStatus, String> {
    let codex_home = codex_account::get_codex_home();
    let source_root = ensure_router_source_for_install()?;
    let output = run_router_shell_command(&source_root, &codex_home, "install")?;
    router_action_succeeded(&output, "安装")?;
    Ok(get_status())
}

pub fn update() -> Result<CodexRouterStatus, String> {
    let (codex_home, source_root, _) = require_router_source()?;
    let output = run_node_router_command(&source_root, &codex_home, "update.mjs", &["update"])?;
    router_action_succeeded(&output, "更新")?;
    Ok(get_status())
}

pub fn enable() -> Result<CodexRouterStatus, String> {
    let (codex_home, source_root, _) = require_router_source()?;
    if codex_account::is_cockpit_local_access_routing_active(&codex_home) {
        return Err("Cockpit API Service 正在接管当前 Codex Profile；请先停用该服务再启用 Codex Router".to_string());
    }
    let output = run_router_shell_command(&source_root, &codex_home, "enable")?;
    router_action_succeeded(&output, "启用")?;
    Ok(get_status())
}

pub fn disable() -> Result<CodexRouterStatus, String> {
    let (codex_home, source_root, _) = require_router_source()?;
    let output = run_router_shell_command(&source_root, &codex_home, "disable")?;
    router_action_succeeded(&output, "停用")?;
    Ok(get_status())
}

pub fn list_providers() -> Result<Vec<CodexRouterProvider>, String> {
    let (codex_home, source_root, _) = require_router_source()?;
    let output = run_node_router_command(&source_root, &codex_home, "control.mjs", &["providers"])?;
    router_action_succeeded(&output, "读取 Provider 状态")?;
    let snapshot = serde_json::from_slice::<RouterProviderSnapshot>(&output.stdout)
        .map_err(|_| "Codex Router 返回了无法识别的 Provider 状态".to_string())?;
    let visible = list_provider_visibility(&source_root, &codex_home)?;
    Ok(snapshot
        .providers
        .into_iter()
        .map(|provider| CodexRouterProvider {
            visible: visible.iter().any(|entry| entry.0 == provider.id && entry.1),
            id: provider.id,
            display_name: provider.display_name,
            kind: provider.kind,
            configured: provider.configured,
            cli_installed: provider.cli_installed,
            cli_runnable: provider.cli_runnable,
            action: provider.action,
        })
        .collect())
}

fn list_provider_visibility(source_root: &Path, codex_home: &Path) -> Result<Vec<(String, bool)>, String> {
    #[derive(Deserialize)]
    struct Snapshot {
        providers: Vec<Entry>,
    }
    #[derive(Deserialize)]
    struct Entry {
        id: String,
        visible: bool,
    }
    let output = run_node_router_command(source_root, codex_home, "providers.mjs", &["list", "--json"])?;
    router_action_succeeded(&output, "读取 Provider 可见性")?;
    let snapshot = serde_json::from_slice::<Snapshot>(&output.stdout)
        .map_err(|_| "Codex Router 返回了无法识别的 Provider 可见性".to_string())?;
    Ok(snapshot
        .providers
        .into_iter()
        .map(|provider| (provider.id, provider.visible))
        .collect())
}

pub fn set_provider_enabled(provider_id: &str, enabled: bool) -> Result<Vec<CodexRouterProvider>, String> {
    let provider_id = validate_provider_id(provider_id)?;
    let (codex_home, source_root, _) = require_router_source()?;
    let action = if enabled { "enable" } else { "disable" };
    let output = run_node_router_command(&source_root, &codex_home, "providers.mjs", &[action, provider_id])?;
    router_action_succeeded(&output, "更新 Provider")?;
    list_providers()
}

pub fn install_provider_cli(provider_id: &str) -> Result<Vec<CodexRouterProvider>, String> {
    let provider_id = validate_provider_id(provider_id)?;
    let (codex_home, source_root, _) = require_router_source()?;
    let output = run_node_router_command(&source_root, &codex_home, "control.mjs", &["install-cli", provider_id])?;
    router_action_succeeded(&output, "安装 Provider CLI")?;
    list_providers()
}

pub fn login_provider(provider_id: &str) -> Result<Vec<CodexRouterProvider>, String> {
    let provider_id = validate_provider_id(provider_id)?;
    let (codex_home, source_root, _) = require_router_source()?;
    let output = run_node_router_command(&source_root, &codex_home, "control.mjs", &["login", provider_id])?;
    router_action_succeeded(&output, "Provider 登录")?;
    list_providers()
}

pub fn run_doctor() -> Result<CodexRouterDoctorReport, String> {
    let (codex_home, source_root, _) = require_router_source()?;
    let output = run_node_router_command(&source_root, &codex_home, "doctor.mjs", &["--json"])?;
    let snapshot = serde_json::from_slice::<RouterDoctorSnapshot>(&output.stdout)
        .map_err(|_| "Codex Router 未返回可读取的诊断报告".to_string())?;
    Ok(CodexRouterDoctorReport {
        ok: snapshot.ok,
        checks: snapshot
            .checks
            .into_iter()
            .map(|check| CodexRouterDoctorCheck {
                status: check.status,
                name: check.name,
                detail: check.detail,
                fix: check.fix,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::{enabled_provider_count, RouterInstallManifest, RouterInstallManifestCurrent};

    #[test]
    fn counts_manifest_provider_entries_without_reading_credentials() {
        let current = RouterInstallManifestCurrent {
            source_root: Some("/tmp/router".to_string()),
            package_version: Some("0.4.0".to_string()),
            providers: Some(serde_json::json!([{"id": "grok-oauth"}, {"id": "ollama"}])),
        };
        assert_eq!(enabled_provider_count(Some(&current)), 2);
        assert_eq!(enabled_provider_count(None), 0);

        let manifest: RouterInstallManifest = serde_json::from_value(serde_json::json!({
            "version": 1,
            "current": { "sourceRoot": "/tmp/router", "packageVersion": "0.4.0" }
        }))
        .expect("parse manifest");
        assert_eq!(manifest.current.unwrap().package_version.as_deref(), Some("0.4.0"));
    }
}
