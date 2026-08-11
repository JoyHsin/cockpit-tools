//! Safe, narrow integration with an existing Codex Router installation.
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

fn router_state_dir(codex_home: &Path) -> PathBuf {
    codex_home.join(ROUTER_STATE_DIR)
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

fn status_from_manifest(
    codex_home: &Path,
    state_dir: &Path,
    manifest: Option<&RouterInstallManifest>,
    last_error: Option<String>,
) -> CodexRouterStatus {
    let current = manifest.and_then(|entry| entry.current.as_ref());
    let installed = manifest.and_then(router_source_root).is_some();
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

    let codex_home = codex_account::get_codex_home();
    let state_dir = router_state_dir(&codex_home);
    let manifest = read_install_manifest(&state_dir)
        .ok_or("未检测到受支持的 Codex Router 安装记录")?;
    let source_root = router_source_root(&manifest)
        .ok_or("Codex Router 安装目录不可用；请使用 Router 上游工具修复安装")?;

    let output = Command::new(node_binary())
        .arg(source_root.join("src").join("service.mjs"))
        .arg(action)
        .current_dir(&source_root)
        .output()
        .map_err(|_| "无法启动 Codex Router 服务控制命令；请检查 Node.js 运行时".to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Codex Router 服务{}失败（退出码 {}）",
            action,
            output.status.code().unwrap_or(-1)
        ));
    }

    Ok(status_from_manifest(
        &codex_home,
        &state_dir,
        Some(&manifest),
        None,
    ))
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
