//! Cockpit Unified Model Gateway: official Codex passthrough plus Grok OAuth
//! and API providers on a single bundled cliproxy sidecar.

use crate::models::unified_model_gateway::{
    UnifiedAccessScope, UnifiedApiProviderDraft, UnifiedCredentialKind, UnifiedCredentialRef,
    UnifiedGatewayBackupRef, UnifiedGatewayCatalogEntry, UnifiedGatewayConflict,
    UnifiedGatewayDiagnostics, UnifiedGatewayLifecycle, UnifiedGatewayLogEvent,
    UnifiedGatewayProfileState, UnifiedGatewayStateView, UnifiedGatewayStatus, UnifiedGatewayStore,
    UnifiedGrokAccountOption, UnifiedModel, UnifiedModelCapabilities, UnifiedModelProvider,
    UnifiedProviderHealth, UnifiedProviderType, UnifiedRouterMigrationPreview,
    UnifiedRouterMigrationProvider, UnifiedRoutingPolicy, UnifiedSharedSessionBinding,
    UNIFIED_GATEWAY_ENDPOINT_MARKER, UNIFIED_GATEWAY_MODEL_CATALOG_FILE,
    UNIFIED_GATEWAY_PROTOCOL_VERSION, UNIFIED_GATEWAY_STATE_FILE, UNIFIED_GATEWAY_STORE_VERSION,
};
use crate::modules::atomic_write::write_string_atomic;
use crate::modules::credential_broker::BrokerLaunchSecrets;
use crate::modules::{
    account, codex_account, codex_config_format, codex_local_access, codex_protocol, codex_router,
    credential_broker, grok_account, logger,
};
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::Mutex as TokioMutex;
use toml_edit::{value, Document};
use url::Url;

const STORE_FILE: &str = "unified_model_gateway.json";
const BACKUP_DIR: &str = "unified_gateway_backups";
const SIDECAR_DIR: &str = "unified_gateway_sidecar";
const OFFICIAL_PROVIDER_ID: &str = "official-codex";
const GROK_OAUTH_PROVIDER_ID: &str = "grok-oauth";
const XAI_API_PROVIDER_ID: &str = "xai-api";
// This provider deliberately keeps Codex's ChatGPT authentication while
// disabling the Responses WebSocket transport. The latter cannot inspect the
// selected model until after the connection is upgraded, so it cannot safely
// choose an external provider.
const UNIFIED_GATEWAY_CODEX_PROVIDER_ID: &str = "cockpit-unified-gateway";
const UNIFIED_GATEWAY_CODEX_PROVIDER_NAME: &str = "Cockpit Unified Gateway (ChatGPT auth)";
const DEFAULT_PORT: u16 = 1457;
const QUOTA_TTL_MS: i64 = 10 * 60 * 1000;
const SUPPORTED_CODEX_RANGE: &str = "verified-codex-app-cli-first-release";
const THREAT_MODEL_NOTE: &str =
    "Prevents network, other OS users, and accidental local programs from using the gateway. A malicious process running as the same OS user that can read Cockpit's private config is outside this desktop app's isolated threat model.";
const MANAGED_KEYS: &[&str] = &["openai_base_url", "model_catalog_json"];

static STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static RUNTIME: OnceLock<TokioMutex<GatewayRuntime>> = OnceLock::new();

#[derive(Default)]
struct GatewayRuntime {
    running: bool,
    last_error: Option<String>,
    child_pid: Option<u32>,
    events: Vec<UnifiedGatewayLogEvent>,
}

fn store_lock() -> &'static Mutex<()> {
    STORE_LOCK.get_or_init(|| Mutex::new(()))
}

fn runtime() -> &'static TokioMutex<GatewayRuntime> {
    RUNTIME.get_or_init(|| TokioMutex::new(GatewayRuntime::default()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn store_path() -> Result<PathBuf, String> {
    Ok(account::get_data_dir()?.join(STORE_FILE))
}

fn backup_root() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join(BACKUP_DIR);
    std::fs::create_dir_all(&path).map_err(|error| format!("创建网关备份目录失败: {error}"))?;
    Ok(path)
}

fn sidecar_dir() -> Result<PathBuf, String> {
    let path = account::get_data_dir()?.join(SIDECAR_DIR);
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("创建统一网关 sidecar 目录失败: {error}"))?;
    Ok(path)
}

fn sha256_hex(content: &str) -> String {
    format!("{:x}", Sha256::digest(content.as_bytes()))
}

fn random_token(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

fn default_grok_models() -> Vec<UnifiedModel> {
    ["grok-4.5", "grok-4.6"]
        .into_iter()
        .map(|id| UnifiedModel {
            id: format!("{GROK_OAUTH_PROVIDER_ID}/{id}"),
            display_name: if id == "grok-4.5" {
                "Grok 4.5 (OAuth)".to_string()
            } else {
                "Grok 4.6 (OAuth)".to_string()
            },
            provider_id: GROK_OAUTH_PROVIDER_ID.to_string(),
            upstream_model: id.to_string(),
            route: "grok-oauth".to_string(),
            capabilities: UnifiedModelCapabilities {
                text: true,
                streaming: true,
                tools: true,
                vision: true,
                search: true,
            },
            reasoning_efforts: vec!["low".to_string(), "medium".to_string(), "high".to_string()],
            context_window: Some(256_000),
            enabled: false,
            availability: "available".to_string(),
        })
        .collect()
}

fn default_providers() -> Vec<UnifiedModelProvider> {
    vec![
        UnifiedModelProvider {
            id: OFFICIAL_PROVIDER_ID.to_string(),
            provider_type: UnifiedProviderType::OfficialCodex,
            display_name: "GPT / Codex 官方账号".to_string(),
            enabled: true,
            ..UnifiedModelProvider::default()
        },
        UnifiedModelProvider {
            id: GROK_OAUTH_PROVIDER_ID.to_string(),
            provider_type: UnifiedProviderType::GrokOauth,
            display_name: "Grok (OAuth)".to_string(),
            enabled: false,
            ..UnifiedModelProvider::default()
        },
        UnifiedModelProvider {
            id: XAI_API_PROVIDER_ID.to_string(),
            provider_type: UnifiedProviderType::XaiApi,
            display_name: "xAI API".to_string(),
            enabled: false,
            base_url: Some("https://api.x.ai".to_string()),
            wire_api: Some("chat_completions".to_string()),
            ..UnifiedModelProvider::default()
        },
    ]
}

fn official_models() -> Vec<UnifiedModel> {
    let mut seen = HashSet::new();
    let mut models = Vec::new();
    for id in codex_protocol::managed_codex_model_ids() {
        let key = id.to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        models.push(UnifiedModel {
            id: id.clone(),
            display_name: id.clone(),
            provider_id: OFFICIAL_PROVIDER_ID.to_string(),
            upstream_model: id.clone(),
            route: "official".to_string(),
            capabilities: UnifiedModelCapabilities {
                text: true,
                streaming: true,
                tools: true,
                vision: false,
                search: false,
            },
            reasoning_efforts: vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()],
            context_window: None,
            enabled: true,
            availability: "available".to_string(),
        });
    }
    models
}

fn migrate_store(mut store: UnifiedGatewayStore) -> UnifiedGatewayStore {
    if store.version == 0 {
        store.version = UNIFIED_GATEWAY_STORE_VERSION;
    }
    if store.ownership_id.trim().is_empty() {
        store.ownership_id = format!("ugw_{}", random_token(16));
    }
    if store.port == 0 {
        store.port = DEFAULT_PORT;
    }
    if store.client_base_url_host.trim().is_empty() {
        store.client_base_url_host = "127.0.0.1".to_string();
    }
    if store.capability_token.trim().is_empty() {
        store.capability_token = random_token(24);
    }
    if store.providers.is_empty() {
        store.providers = default_providers();
    }
    if !store
        .providers
        .iter()
        .any(|item| item.id == OFFICIAL_PROVIDER_ID)
    {
        store.providers.insert(0, default_providers()[0].clone());
    }
    if !store
        .providers
        .iter()
        .any(|item| item.id == GROK_OAUTH_PROVIDER_ID)
    {
        store.providers.push(default_providers()[1].clone());
    }
    let official_ids = official_models()
        .into_iter()
        .map(|model| model.id)
        .collect::<HashSet<_>>();
    let mut next_models = official_models();
    for model in store.models.drain(..) {
        if official_ids.contains(&model.id) && model.provider_id == OFFICIAL_PROVIDER_ID {
            continue;
        }
        next_models.push(namespaced_model(model, &official_ids));
    }
    if !next_models
        .iter()
        .any(|model| model.provider_id == GROK_OAUTH_PROVIDER_ID)
    {
        next_models.extend(default_grok_models());
    }
    store.models = next_models;
    if store.created_at == 0 {
        store.created_at = now_ms();
    }
    store.updated_at = now_ms();
    store
}

fn namespaced_model(mut model: UnifiedModel, official_ids: &HashSet<String>) -> UnifiedModel {
    if model.provider_id != OFFICIAL_PROVIDER_ID {
        let provider_prefix = format!("{}/", model.provider_id.trim());
        let upstream = if model.upstream_model.trim().is_empty() {
            model.id.trim().to_string()
        } else {
            model.upstream_model.trim().to_string()
        };
        if !model.id.starts_with(&provider_prefix) {
            model.id = format!("{provider_prefix}{upstream}");
        }
        if official_ids.contains(&model.id) {
            model.id = format!("{provider_prefix}{upstream}");
        }
    }
    model
}

pub fn load_store() -> Result<UnifiedGatewayStore, String> {
    let _lock = store_lock()
        .lock()
        .map_err(|_| "获取统一网关存储锁失败".to_string())?;
    load_store_locked()
}

fn load_store_locked() -> Result<UnifiedGatewayStore, String> {
    let path = store_path()?;
    if !path.exists() {
        return Ok(migrate_store(UnifiedGatewayStore {
            version: UNIFIED_GATEWAY_STORE_VERSION,
            lifecycle: UnifiedGatewayLifecycle::Disabled,
            port: DEFAULT_PORT,
            providers: default_providers(),
            models: {
                let mut models = official_models();
                models.extend(default_grok_models());
                models
            },
            created_at: now_ms(),
            updated_at: now_ms(),
            ..UnifiedGatewayStore::default()
        }));
    }
    let content =
        std::fs::read_to_string(&path).map_err(|error| format!("读取统一网关配置失败: {error}"))?;
    let store = serde_json::from_str::<UnifiedGatewayStore>(&content)
        .map_err(|error| format!("解析统一网关配置失败: {error}"))?;
    Ok(migrate_store(store))
}

fn save_store_locked(store: &UnifiedGatewayStore) -> Result<(), String> {
    let path = store_path()?;
    let content = serde_json::to_string_pretty(store)
        .map_err(|error| format!("序列化统一网关配置失败: {error}"))?;
    write_string_atomic(&path, &content)
        .map_err(|error| format!("写入统一网关配置失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn save_store(store: &UnifiedGatewayStore) -> Result<(), String> {
    let _lock = store_lock()
        .lock()
        .map_err(|_| "获取统一网关存储锁失败".to_string())?;
    save_store_locked(store)
}

pub fn record_shared_session_binding(
    account_id: String,
    source_path: String,
    identity: String,
    credential_fingerprint: String,
) -> Result<(), String> {
    let mut store = load_store()?;
    store
        .shared_session_bindings
        .retain(|item| item.account_id != account_id && item.source_path != source_path);
    store
        .shared_session_bindings
        .push(UnifiedSharedSessionBinding {
            account_id,
            source_path,
            identity,
            credential_fingerprint,
            last_synced_at: Some(now_ms()),
            paused: false,
            pause_reason: None,
        });
    store.updated_at = now_ms();
    save_store(&store)
}

pub fn mirror_shared_session_after_refresh(account: &crate::models::grok::GrokAccount) {
    let Ok(mut store) = load_store() else {
        return;
    };
    let Some(index) = store
        .shared_session_bindings
        .iter()
        .position(|item| item.account_id == account.id && !item.paused)
    else {
        return;
    };
    let binding = store.shared_session_bindings[index].clone();
    match grok_account::mirror_shared_session_source(
        account,
        Path::new(&binding.source_path),
        &binding.identity,
        &binding.credential_fingerprint,
    ) {
        Ok(true) => {
            if let Some(item) = store.shared_session_bindings.get_mut(index) {
                item.credential_fingerprint = credential_broker::fingerprint_secret(
                    account.refresh_token.as_deref().unwrap_or_default(),
                );
                item.last_synced_at = Some(now_ms());
            }
            let _ = save_store(&store);
        }
        Ok(false) => {
            push_event(
                "warn",
                "shared_session_adopted",
                "共享会话源已被其他进程轮换，已暂停自动回写",
            );
        }
        Err(error) => {
            if let Some(item) = store.shared_session_bindings.get_mut(index) {
                item.paused = true;
                item.pause_reason = Some(error.clone());
            }
            let _ = save_store(&store);
            push_event("warn", "shared_session_paused", &error);
        }
    }
}

fn profile_dir() -> PathBuf {
    codex_account::get_codex_home()
}

fn config_path(profile: &Path) -> PathBuf {
    profile.join("config.toml")
}

fn profile_state_path(profile: &Path) -> PathBuf {
    profile.join(UNIFIED_GATEWAY_STATE_FILE)
}

fn catalog_path(profile: &Path) -> PathBuf {
    profile.join(UNIFIED_GATEWAY_MODEL_CATALOG_FILE)
}

pub fn capability_base_url(store: &UnifiedGatewayStore) -> String {
    format!(
        "http://{}:{}{}/{}/v1",
        store.client_base_url_host.trim(),
        store.port,
        UNIFIED_GATEWAY_ENDPOINT_MARKER.trim_end_matches('/'),
        store.capability_token.trim()
    )
}

pub fn is_unified_gateway_signed_routing_active(base_dir: &Path) -> bool {
    let Ok(content) = std::fs::read_to_string(config_path(base_dir)) else {
        return false;
    };
    let Ok(doc) = codex_config_format::read_codex_config_doc_from_str(&content) else {
        return false;
    };
    ownership_matches(base_dir, &doc).is_ok_and(|matched| matched)
}

fn read_profile_state(profile: &Path) -> Option<UnifiedGatewayProfileState> {
    let content = std::fs::read_to_string(profile_state_path(profile)).ok()?;
    serde_json::from_str::<UnifiedGatewayProfileState>(&content).ok()
}

fn ownership_matches(profile: &Path, doc: &Document) -> Result<bool, String> {
    let store = load_store()?;
    let Some(state) = read_profile_state(profile) else {
        return Ok(false);
    };
    if state.ownership_id != store.ownership_id {
        return Ok(false);
    }
    let configured = doc
        .get("openai_base_url")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let expected = capability_base_url(&store);
    if state.ownership_id != store.ownership_id || state.managed_base_url != expected {
        return Ok(false);
    }
    if state.mode == "root-openai" && state.managed_provider == "openai" {
        // Compatibility with the first Unified Gateway release. Startup
        // upgrades this shape before a new Codex process is launched.
        return Ok(configured == expected);
    }
    Ok(unified_gateway_provider_matches(doc, &expected)
        && state.mode == "custom-provider"
        && state.managed_provider == UNIFIED_GATEWAY_CODEX_PROVIDER_ID)
}

fn hash_managed_fields(doc: &Document) -> String {
    let base = doc
        .get("openai_base_url")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let catalog = doc
        .get("model_catalog_json")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let provider_table = doc
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(UNIFIED_GATEWAY_CODEX_PROVIDER_ID))
        .and_then(|item| item.as_table());
    let provider_base = provider_table
        .and_then(|table| table.get("base_url"))
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let wire_api = provider_table
        .and_then(|table| table.get("wire_api"))
        .and_then(|item| item.as_str())
        .unwrap_or_default();
    let requires_openai_auth = provider_table
        .and_then(|table| table.get("requires_openai_auth"))
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    let supports_websockets = provider_table
        .and_then(|table| table.get("supports_websockets"))
        .and_then(|item| item.as_bool())
        .unwrap_or(true);
    sha256_hex(&format!(
        "{base}|{catalog}|{provider}|{provider_base}|{wire_api}|{requires_openai_auth}|{supports_websockets}"
    ))
}

fn unified_gateway_provider_matches(doc: &Document, expected_base_url: &str) -> bool {
    if doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        != Some(UNIFIED_GATEWAY_CODEX_PROVIDER_ID)
    {
        return false;
    }
    let Some(table) = doc
        .get("model_providers")
        .and_then(|item| item.as_table())
        .and_then(|providers| providers.get(UNIFIED_GATEWAY_CODEX_PROVIDER_ID))
        .and_then(|item| item.as_table())
    else {
        return false;
    };
    table.get("base_url").and_then(|item| item.as_str()) == Some(expected_base_url)
        && table.get("wire_api").and_then(|item| item.as_str()) == Some("responses")
        && table
            .get("requires_openai_auth")
            .and_then(|item| item.as_bool())
            == Some(true)
        && table
            .get("supports_websockets")
            .and_then(|item| item.as_bool())
            == Some(false)
}

fn push_event(level: &str, code: &str, message: &str) {
    logger::log_info(&format!(
        "[UnifiedGateway] {} {}",
        code,
        credential_broker::redact_text(message)
    ));
    if let Ok(mut runtime) = runtime().try_lock() {
        runtime.events.push(UnifiedGatewayLogEvent {
            at: now_ms(),
            level: level.to_string(),
            code: code.to_string(),
            message: credential_broker::redact_text(message),
        });
        if runtime.events.len() > 40 {
            let drain = runtime.events.len() - 40;
            runtime.events.drain(0..drain);
        }
    }
}

fn local_access_enabled() -> bool {
    // Best-effort: if API Service owns the profile we refuse to take over.
    let profile = profile_dir();
    codex_account::is_cockpit_local_access_routing_active(&profile)
}

fn router_active() -> bool {
    let profile = profile_dir();
    codex_account::is_codex_router_signed_routing_active(&profile)
}

fn ensure_exclusive_owner() -> Result<(), String> {
    if local_access_enabled() {
        return Err(
            "Cockpit API Service 正在接管当前 Codex Profile。请先停用 API Service，再启用统一模型网关。"
                .to_string(),
        );
    }
    if router_active() {
        return Err(
            "旧 Codex Router 正在接管当前 Codex Profile。请先迁移或停用 Router，再启用统一模型网关。"
                .to_string(),
        );
    }
    Ok(())
}

pub fn assert_not_active_for_other_owner() -> Result<(), String> {
    let store = load_store()?;
    if matches!(
        store.lifecycle,
        UnifiedGatewayLifecycle::Active
            | UnifiedGatewayLifecycle::Verifying
            | UnifiedGatewayLifecycle::Configured
    ) {
        return Err(
            "统一模型网关正在接管当前 Codex Profile。请先停用统一网关，再启用其他路由。"
                .to_string(),
        );
    }
    Ok(())
}

fn build_catalog_json(store: &UnifiedGatewayStore) -> Result<String, String> {
    let official_ids = store
        .models
        .iter()
        .filter(|model| model.enabled && model.provider_id == OFFICIAL_PROVIDER_ID)
        .map(|model| model.id.clone())
        .collect::<Vec<_>>();
    let custom = store
        .models
        .iter()
        .filter(|model| model.enabled && model.provider_id != OFFICIAL_PROVIDER_ID)
        .map(|model| (model.id.clone(), model.display_name.clone()))
        .collect::<Vec<_>>();
    let mut models = Vec::new();
    if !official_ids.is_empty() {
        let official = codex_protocol::build_codex_client_models_response(&official_ids);
        if let Some(list) = official.get("models").and_then(Value::as_array) {
            models.extend(list.clone());
        }
    }
    if !custom.is_empty() {
        let custom_response =
            codex_protocol::build_codex_client_models_response_with_model_definitions(&custom);
        if let Some(list) = custom_response.get("models").and_then(Value::as_array) {
            models.extend(list.clone());
        }
    }
    let custom_by_id = store
        .models
        .iter()
        .filter(|model| model.enabled && model.provider_id != OFFICIAL_PROVIDER_ID)
        .map(|model| {
            (
                model.id.as_str(),
                codex_protocol::UnifiedCodexModelMetadata {
                    display_name: model.display_name.clone(),
                    reasoning_efforts: model.reasoning_efforts.clone(),
                    context_window: model.context_window,
                    capabilities: model.capabilities.clone(),
                    availability: model.availability.clone(),
                },
            )
        })
        .collect::<std::collections::HashMap<_, _>>();
    let official: HashSet<_> = official_ids.into_iter().collect();
    for model in &mut models {
        let Some(object) = model.as_object_mut() else {
            continue;
        };
        let id = object
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| object.get("slug").and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        if let Some(metadata) = custom_by_id.get(id.as_str()) {
            codex_protocol::apply_unified_model_metadata(object, metadata);
        }
        object.insert(
            "prefer_websockets".to_string(),
            Value::Bool(official.contains(&id)),
        );
    }
    let catalog = json!({
        "models": models,
    });
    serde_json::to_string_pretty(&catalog).map_err(|error| format!("生成模型目录失败: {error}"))
}

fn write_catalog(profile: &Path, store: &UnifiedGatewayStore) -> Result<(), String> {
    let content = build_catalog_json(store)?;
    write_string_atomic(&catalog_path(profile), &content)
        .map_err(|error| format!("写入模型目录失败: {error}"))?;
    let _ = codex_local_access::invalidate_codex_model_cache(profile);
    Ok(())
}

fn backup_profile(profile: &Path, store: &mut UnifiedGatewayStore) -> Result<(), String> {
    let backup_id = format!("bak_{}", random_token(12));
    let dest = backup_root()?.join(&backup_id);
    std::fs::create_dir_all(&dest).map_err(|error| format!("创建备份目录失败: {error}"))?;
    let config = config_path(profile);
    let backup_content = if config.exists() {
        let existing = std::fs::read_to_string(&config)
            .map_err(|error| format!("读取 config.toml 失败: {error}"))?;
        stripped_config_content(&existing)?
    } else {
        String::new()
    };
    if !backup_content.is_empty() {
        std::fs::write(dest.join("config.toml"), &backup_content)
            .map_err(|error| format!("备份 config.toml 失败: {error}"))?;
    }
    store.backup = Some(UnifiedGatewayBackupRef {
        backup_id,
        original_config_hash: sha256_hex(&backup_content),
        managed_config_hash: String::new(),
        created_at: now_ms(),
        profile_dir: profile.display().to_string(),
    });
    Ok(())
}

fn apply_managed_config(profile: &Path, store: &UnifiedGatewayStore) -> Result<String, String> {
    let path = config_path(profile);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut doc = if existing.trim().is_empty() {
        Document::new()
    } else {
        codex_config_format::read_codex_config_doc_from_str(&existing)
            .map_err(|error| format!("解析 config.toml 失败: {error}"))?
    };
    let provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if provider.is_some_and(|value| value != "openai" && value != UNIFIED_GATEWAY_CODEX_PROVIDER_ID)
    {
        return Err(
            "当前 Codex Profile 使用了自定义 Provider，统一网关不会覆盖该配置。".to_string(),
        );
    }
    // A root `openai_base_url` leaves the built-in OpenAI provider's
    // WebSocket capability enabled. Codex then connects before it knows the
    // target model, and the relay can only forward it to ChatGPT. Use a
    // dedicated provider instead so the client keeps ChatGPT auth but always
    // uses the HTTP/SSE transport where routing is model-aware.
    let _ = doc.remove("openai_base_url");
    doc["model_provider"] = value(UNIFIED_GATEWAY_CODEX_PROVIDER_ID);
    if doc.get("model_providers").is_none() {
        doc["model_providers"] = toml_edit::table();
    }
    let providers = doc["model_providers"]
        .as_table_mut()
        .ok_or("config.toml 中 model_providers 不是合法表结构")?;
    if !providers.contains_key(UNIFIED_GATEWAY_CODEX_PROVIDER_ID) {
        providers[UNIFIED_GATEWAY_CODEX_PROVIDER_ID] = toml_edit::table();
    }
    let provider_table = providers[UNIFIED_GATEWAY_CODEX_PROVIDER_ID]
        .as_table_mut()
        .ok_or("config.toml 中统一网关 provider 不是合法表结构")?;
    provider_table["name"] = value(UNIFIED_GATEWAY_CODEX_PROVIDER_NAME);
    provider_table["base_url"] = value(capability_base_url(store));
    provider_table["wire_api"] = value("responses");
    provider_table["requires_openai_auth"] = value(true);
    provider_table["supports_websockets"] = value(false);
    // Newer Codex requires an absolute path here; a bare filename fails with
    // "AbsolutePathBuf deserialized without a base path".
    doc["model_catalog_json"] = value(catalog_path(profile).to_string_lossy().to_string());
    // Built-in `openai` already sends official auth. Overriding
    // `[model_providers.openai]` is rejected as a reserved provider ID.
    remove_reserved_openai_provider_override(&mut doc);
    let content = codex_config_format::codex_config_doc_to_string(&mut doc);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("创建 Codex 目录失败: {error}"))?;
    }
    codex_config_format::write_codex_config_toml_atomic(&path, &content)
        .map_err(|error| format!("写入 config.toml 失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(hash_managed_fields(&doc))
}

fn write_profile_state(
    profile: &Path,
    store: &UnifiedGatewayStore,
    managed_hash: &str,
) -> Result<(), String> {
    let state = UnifiedGatewayProfileState {
        version: 1,
        ownership_id: store.ownership_id.clone(),
        mode: "custom-provider".to_string(),
        managed_provider: UNIFIED_GATEWAY_CODEX_PROVIDER_ID.to_string(),
        managed_base_url: capability_base_url(store),
        catalog_file: catalog_path(profile).to_string_lossy().to_string(),
        original_config_hash: store
            .backup
            .as_ref()
            .map(|item| item.original_config_hash.clone())
            .unwrap_or_default(),
        managed_config_hash: managed_hash.to_string(),
        updated_at: now_ms(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|error| format!("序列化网关状态失败: {error}"))?;
    write_string_atomic(&profile_state_path(profile), &content)
        .map_err(|error| format!("写入 unified-gateway-state.json 失败: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(
            profile_state_path(profile),
            std::fs::Permissions::from_mode(0o600),
        );
    }
    Ok(())
}

fn remove_managed_fields(doc: &mut Document) {
    for key in MANAGED_KEYS {
        let _ = doc.remove(key);
    }
    let remove_root_provider = doc
        .get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        == Some(UNIFIED_GATEWAY_CODEX_PROVIDER_ID);
    if remove_root_provider {
        let _ = doc.remove("model_provider");
    }
    let remove_root_base_url = doc
        .get("openai_base_url")
        .and_then(|item| item.as_str())
        .is_some_and(|value| value.contains(UNIFIED_GATEWAY_ENDPOINT_MARKER));
    if remove_root_base_url {
        let _ = doc.remove("openai_base_url");
    }
    remove_unified_gateway_provider_override(doc);
    remove_reserved_openai_provider_override(doc);
}

fn config_has_managed_fields(doc: &Document) -> bool {
    MANAGED_KEYS.iter().any(|key| doc.get(key).is_some())
        || doc
            .get("openai_base_url")
            .and_then(|item| item.as_str())
            .is_some_and(|value| value.contains(UNIFIED_GATEWAY_ENDPOINT_MARKER))
        || doc
            .get("model_provider")
            .and_then(|item| item.as_str())
            .map(str::trim)
            == Some(UNIFIED_GATEWAY_CODEX_PROVIDER_ID)
        || doc
            .get("model_providers")
            .and_then(|item| item.get("openai"))
            .is_some()
        || doc
            .get("model_providers")
            .and_then(|item| item.get(UNIFIED_GATEWAY_CODEX_PROVIDER_ID))
            .is_some()
}

fn stripped_config_content(existing: &str) -> Result<String, String> {
    if existing.trim().is_empty() {
        return Ok(existing.to_string());
    }
    let mut doc = codex_config_format::read_codex_config_doc_from_str(existing)
        .map_err(|error| format!("解析 config.toml 失败: {error}"))?;
    if !config_has_managed_fields(&doc) {
        return Ok(existing.to_string());
    }
    remove_managed_fields(&mut doc);
    Ok(codex_config_format::codex_config_doc_to_string(&mut doc))
}

fn strip_managed_fields_from_config(profile: &Path) -> Result<(), String> {
    let path = config_path(profile);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let content = stripped_config_content(&existing)?;
    if content == existing {
        return Ok(());
    }
    codex_config_format::write_codex_config_toml_atomic(&path, &content)
        .map_err(|error| format!("移除网关字段失败: {error}"))?;
    Ok(())
}

fn remove_reserved_openai_provider_override(doc: &mut Document) {
    let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_mut())
    else {
        return;
    };
    let _ = providers.remove("openai");
    if providers.is_empty() {
        let _ = doc.remove("model_providers");
    }
}

fn remove_unified_gateway_provider_override(doc: &mut Document) {
    let Some(providers) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_mut())
    else {
        return;
    };
    let _ = providers.remove(UNIFIED_GATEWAY_CODEX_PROVIDER_ID);
    if providers.is_empty() {
        let _ = doc.remove("model_providers");
    }
}

fn restore_official_mode(
    profile: &Path,
    store: &UnifiedGatewayStore,
    force_backup: bool,
) -> Result<(), String> {
    let path = config_path(profile);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let current_hash = sha256_hex(&existing);
    let managed_hash = store
        .backup
        .as_ref()
        .map(|item| item.managed_config_hash.clone())
        .unwrap_or_default();
    let original_hash = store
        .backup
        .as_ref()
        .map(|item| item.original_config_hash.clone())
        .unwrap_or_default();

    if force_backup
        || (!managed_hash.is_empty()
            && current_hash == hash_of_current_or_managed(&existing, &managed_hash))
        || current_hash == original_hash
    {
        if let Some(backup) = store.backup.as_ref() {
            let backup_file = backup_root()?.join(&backup.backup_id).join("config.toml");
            if backup_file.exists()
                && (force_backup
                    || current_hash == original_hash
                    || user_only_has_managed_fields(&existing))
            {
                let backup_content = std::fs::read_to_string(&backup_file)
                    .map_err(|error| format!("读取备份失败: {error}"))?;
                codex_config_format::write_codex_config_toml_atomic(&path, &backup_content)
                    .map_err(|error| format!("恢复 config.toml 失败: {error}"))?;
            } else if !existing.trim().is_empty() {
                let mut doc = codex_config_format::read_codex_config_doc_from_str(&existing)
                    .map_err(|error| format!("解析 config.toml 失败: {error}"))?;
                remove_managed_fields(&mut doc);
                let content = codex_config_format::codex_config_doc_to_string(&mut doc);
                codex_config_format::write_codex_config_toml_atomic(&path, &content)
                    .map_err(|error| format!("移除网关字段失败: {error}"))?;
            }
        }
    } else {
        return Err("CONFIG_CONFLICT".to_string());
    }

    // Always strip gateway keys after restore. Backups taken while the gateway
    // was already managing config.toml can still contain model_catalog_json;
    // deleting the catalog file without removing that key makes Codex fail with
    // "failed to resolve feature override precedence: No such file or directory".
    strip_managed_fields_from_config(profile)?;
    let _ = std::fs::remove_file(profile_state_path(profile));
    let _ = std::fs::remove_file(catalog_path(profile));
    let _ = codex_local_access::invalidate_codex_model_cache(profile);
    Ok(())
}

fn hash_of_current_or_managed(existing: &str, managed_hash: &str) -> String {
    if let Ok(doc) = codex_config_format::read_codex_config_doc_from_str(existing) {
        let managed = hash_managed_fields(&doc);
        if managed == *managed_hash {
            return sha256_hex(existing);
        }
    }
    String::new()
}

fn user_only_has_managed_fields(existing: &str) -> bool {
    let Ok(doc) = codex_config_format::read_codex_config_doc_from_str(existing) else {
        return false;
    };
    doc.get("openai_base_url")
        .and_then(|item| item.as_str())
        .is_some_and(|value| value.contains(UNIFIED_GATEWAY_ENDPOINT_MARKER))
        || doc
            .get("model_provider")
            .and_then(|item| item.as_str())
            .map(str::trim)
            == Some(UNIFIED_GATEWAY_CODEX_PROVIDER_ID)
}

fn inspect_conflict(profile: &Path, store: &UnifiedGatewayStore) -> UnifiedGatewayConflict {
    let existing = std::fs::read_to_string(config_path(profile)).unwrap_or_default();
    let current_hash = sha256_hex(&existing);
    let managed_hash = store
        .backup
        .as_ref()
        .map(|item| item.managed_config_hash.clone());
    let original_hash = store
        .backup
        .as_ref()
        .map(|item| item.original_config_hash.clone());
    let matches_managed = managed_hash
        .as_deref()
        .is_some_and(|hash| hash_of_current_or_managed(&existing, hash) == current_hash);
    UnifiedGatewayConflict {
        present: !existing.trim().is_empty()
            && !matches_managed
            && original_hash.as_deref() != Some(current_hash.as_str()),
        current_hash: Some(current_hash),
        managed_hash,
        original_hash,
        current_preview: Some(existing.chars().take(1200).collect()),
        managed_preview: store.backup.as_ref().and_then(|backup| {
            std::fs::read_to_string(
                backup_root()
                    .ok()?
                    .join(&backup.backup_id)
                    .join("config.toml"),
            )
            .ok()
            .map(|value| value.chars().take(1200).collect())
        }),
    }
}

fn grok_account_options(store: &UnifiedGatewayStore) -> Vec<UnifiedGrokAccountOption> {
    let selected = store
        .credential_refs
        .iter()
        .filter(|item| item.kind == UnifiedCredentialKind::GrokOauth && item.enabled)
        .filter_map(|item| item.account_id.clone())
        .collect::<HashSet<_>>();
    grok_account::list_accounts_checked()
        .unwrap_or_default()
        .into_iter()
        .map(|account| {
            let remaining = grok_account::lowest_remaining_percent(&account);
            let quota_fresh = account
                .usage_updated_at
                .is_some_and(|at| now_ms().saturating_sub(at) <= QUOTA_TTL_MS);
            let remaining = if quota_fresh { remaining } else { None };
            let oauth = account.auth_mode.as_str() == "oauth";
            let ineligible_reason = if !oauth {
                Some("API Key 账号属于 xAI API，不能加入 Grok (OAuth) 池".to_string())
            } else if account.status.as_deref() == Some("reauth_required") {
                Some("Grok 需要重新授权".to_string())
            } else if account.has_grok_code_access == Some(false) {
                Some("该账号没有 Grok Code 权限".to_string())
            } else {
                None
            };
            UnifiedGrokAccountOption {
                eligible: ineligible_reason.is_none(),
                selected: selected.contains(&account.id),
                account_id: account.id,
                email: account.email,
                auth_mode: account.auth_mode.as_str().to_string(),
                has_grok_code_access: account.has_grok_code_access.unwrap_or(false),
                status: account.status,
                remaining_percent: remaining,
                usage_updated_at: account.usage_updated_at,
                source: "cockpit".to_string(),
                ineligible_reason,
            }
        })
        .collect()
}

fn catalog_entries(store: &UnifiedGatewayStore) -> Vec<UnifiedGatewayCatalogEntry> {
    let official = official_models()
        .into_iter()
        .map(|model| model.id)
        .collect::<HashSet<_>>();
    store
        .models
        .iter()
        .map(|model| UnifiedGatewayCatalogEntry {
            id: model.id.clone(),
            display_name: model.display_name.clone(),
            provider_id: model.provider_id.clone(),
            provider_type: store
                .providers
                .iter()
                .find(|provider| provider.id == model.provider_id)
                .map(|provider| provider.provider_type)
                .unwrap_or(UnifiedProviderType::OpenaiCompatible),
            upstream_model: model.upstream_model.clone(),
            enabled: model.enabled,
            conflict: model.provider_id != OFFICIAL_PROVIDER_ID && official.contains(&model.id),
            capabilities: model.capabilities.clone(),
            availability: model.availability.clone(),
        })
        .collect()
}

fn router_preview() -> UnifiedRouterMigrationPreview {
    let status = codex_router::get_status();
    let providers = codex_router::list_providers().unwrap_or_default();
    UnifiedRouterMigrationPreview {
        router_installed: status.installed,
        router_configured: status.configured,
        router_running: status.running,
        providers: providers
            .into_iter()
            .map(|provider| UnifiedRouterMigrationProvider {
                id: provider.id,
                display_name: provider.display_name,
                kind: provider.kind,
                visible: provider.visible,
                configured: provider.configured,
            })
            .collect(),
        matched_grok_account_ids: grok_account::list_accounts_checked()
            .unwrap_or_default()
            .into_iter()
            .filter(|account| account.auth_mode.as_str() == "oauth")
            .map(|account| account.id)
            .collect(),
        importable_local_grok: grok_account::discover_local_import_candidates()
            .ok()
            .is_some_and(|items| items.iter().any(|item| item.importable)),
        warning: None,
    }
}

fn status_from(store: &UnifiedGatewayStore, running: bool) -> UnifiedGatewayStatus {
    let profile = profile_dir();
    let ownership = std::fs::read_to_string(config_path(&profile))
        .ok()
        .and_then(|content| codex_config_format::read_codex_config_doc_from_str(&content).ok())
        .and_then(|doc| ownership_matches(&profile, &doc).ok())
        .unwrap_or(false);
    UnifiedGatewayStatus {
        lifecycle: store.lifecycle,
        running,
        configured: matches!(
            store.lifecycle,
            UnifiedGatewayLifecycle::Configured
                | UnifiedGatewayLifecycle::Verifying
                | UnifiedGatewayLifecycle::Active
        ),
        official_auth_protected: store.official_auth_protected,
        ownership_matched: ownership,
        port: store.port,
        base_url: store
            .lifecycle
            .eq(&UnifiedGatewayLifecycle::Active)
            .then(|| capability_base_url(store)),
        enabled_provider_count: store.providers.iter().filter(|item| item.enabled).count(),
        enabled_model_count: store.models.iter().filter(|item| item.enabled).count(),
        grok_account_count: store
            .credential_refs
            .iter()
            .filter(|item| item.kind == UnifiedCredentialKind::GrokOauth && item.enabled)
            .count(),
        last_error: store.last_error.clone(),
        last_error_code: store.last_error_code.clone(),
        owner: if store.lifecycle == UnifiedGatewayLifecycle::Active {
            "unified-gateway".to_string()
        } else if local_access_enabled() {
            "api-service".to_string()
        } else if router_active() {
            "codex-router".to_string()
        } else {
            "none".to_string()
        },
        conflict: store.lifecycle == UnifiedGatewayLifecycle::RecoveryRequired,
        router_detected: router_preview().router_installed
            && store.lifecycle != UnifiedGatewayLifecycle::Active,
        api_service_active: local_access_enabled(),
        supported_codex_range: SUPPORTED_CODEX_RANGE.to_string(),
        threat_model_note: THREAT_MODEL_NOTE.to_string(),
    }
}

fn diagnostics_from(
    store: &UnifiedGatewayStore,
    runtime: &GatewayRuntime,
) -> UnifiedGatewayDiagnostics {
    let profile = profile_dir();
    let existing = std::fs::read_to_string(config_path(&profile)).unwrap_or_default();
    let broker_listening = credential_broker::is_broker_configured();
    UnifiedGatewayDiagnostics {
        lifecycle: store.lifecycle,
        running: runtime.running,
        official_auth_protected: auth_json_untouched(&profile),
        ownership_matched: ownership_matches(
            &profile,
            &codex_config_format::read_codex_config_doc_from_str(&existing)
                .unwrap_or_else(|_| Document::new()),
        )
        .unwrap_or(false),
        current_config_hash: Some(sha256_hex(&existing)),
        managed_config_hash: store
            .backup
            .as_ref()
            .map(|item| item.managed_config_hash.clone()),
        original_config_hash: store
            .backup
            .as_ref()
            .map(|item| item.original_config_hash.clone()),
        broker_listening,
        sidecar_running: runtime.running,
        last_error: store.last_error.clone(),
        last_error_code: store.last_error_code.clone(),
        recent_events: runtime.events.clone(),
        provider_health: store
            .providers
            .iter()
            .map(|provider| provider_health(store, provider, runtime.running, broker_listening))
            .collect(),
    }
}

fn provider_health(
    store: &UnifiedGatewayStore,
    provider: &UnifiedModelProvider,
    sidecar_running: bool,
    broker_listening: bool,
) -> UnifiedProviderHealth {
    let mut healthy = provider.enabled && sidecar_running;
    let mut detail = if !provider.enabled {
        "disabled".to_string()
    } else if !sidecar_running {
        "sidecar_not_running".to_string()
    } else {
        "ready".to_string()
    };
    match provider.provider_type {
        UnifiedProviderType::OfficialCodex => {
            if healthy && !auth_json_untouched(&profile_dir()) {
                healthy = false;
                detail = "official_auth_missing".to_string();
            }
        }
        UnifiedProviderType::GrokOauth => {
            let has_credential = store.credential_refs.iter().any(|item| {
                item.enabled
                    && item.kind == UnifiedCredentialKind::GrokOauth
                    && item.account_id.is_some()
                    && provider.credential_ref_ids.iter().any(|id| id == &item.id)
            });
            if healthy && !broker_listening {
                healthy = false;
                detail = "credential_broker_not_running".to_string();
            } else if healthy && !has_credential {
                healthy = false;
                detail = "grok_oauth_account_missing".to_string();
            }
        }
        UnifiedProviderType::XaiApi | UnifiedProviderType::OpenaiCompatible => {
            let has_credential = store.credential_refs.iter().any(|item| {
                item.enabled
                    && item.kind == UnifiedCredentialKind::ApiKey
                    && item.secret_id.is_some()
                    && provider.credential_ref_ids.iter().any(|id| id == &item.id)
            });
            if healthy && !broker_listening {
                healthy = false;
                detail = "credential_broker_not_running".to_string();
            } else if healthy
                && provider
                    .base_url
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
            {
                healthy = false;
                detail = "base_url_missing".to_string();
            } else if healthy && !has_credential {
                healthy = false;
                detail = "api_key_missing".to_string();
            }
        }
    }
    UnifiedProviderHealth {
        provider_id: provider.id.clone(),
        display_name: provider.display_name.clone(),
        healthy,
        detail,
    }
}

fn auth_json_untouched(profile: &Path) -> bool {
    profile.join("auth.json").exists()
}

pub async fn get_state() -> Result<UnifiedGatewayStateView, String> {
    let store = load_store()?;
    let runtime = runtime().lock().await;
    Ok(UnifiedGatewayStateView {
        status: status_from(&store, runtime.running),
        providers: store.providers.clone(),
        models: catalog_entries(&store),
        grok_accounts: grok_account_options(&store),
        import_candidates: grok_account::discover_local_import_candidates().unwrap_or_default(),
        credential_refs: store
            .credential_refs
            .iter()
            .cloned()
            .map(|mut item| {
                item.secret_id = item.secret_id.as_ref().map(|_| "stored".to_string());
                item
            })
            .collect(),
        routing_policy: store.routing_policy.clone(),
        diagnostics: diagnostics_from(&store, &runtime),
        conflict: inspect_conflict(&profile_dir(), &store),
        router_migration: router_preview(),
    })
}

fn fail_store(store: &mut UnifiedGatewayStore, code: &str, error: String) {
    store.lifecycle = UnifiedGatewayLifecycle::RecoveryRequired;
    store.last_error = Some(credential_broker::redact_text(&error));
    store.last_error_code = Some(code.to_string());
    store.updated_at = now_ms();
    push_event("error", code, &error);
}

pub async fn enable() -> Result<UnifiedGatewayStateView, String> {
    let mut store = load_store()?;
    if let Err(error) = enable_inner(&mut store).await {
        fail_store(&mut store, "enable_failed", error.clone());
        let _ = save_store(&store);
        let _ = restore_on_failure(&store).await;
        return Err(error);
    }
    get_state().await
}

async fn enable_inner(store: &mut UnifiedGatewayStore) -> Result<(), String> {
    ensure_exclusive_owner()?;
    let profile = profile_dir();
    store.lifecycle = UnifiedGatewayLifecycle::Preparing;
    store.last_error = None;
    store.last_error_code = None;
    store.official_auth_protected = true;
    if store.capability_token.trim().is_empty() {
        store.capability_token = random_token(24);
    }
    if store.ownership_id.trim().is_empty() {
        store.ownership_id = format!("ugw_{}", random_token(16));
    }
    save_store(store)?;

    backup_profile(&profile, store)?;
    write_catalog(&profile, store)?;
    store.lifecycle = UnifiedGatewayLifecycle::Configured;
    save_store(store)?;

    store.lifecycle = UnifiedGatewayLifecycle::Verifying;
    save_store(store)?;
    start_runtime(store).await?;
    let managed_hash = apply_managed_config(&profile, store)?;
    if let Some(backup) = store.backup.as_mut() {
        backup.managed_config_hash = managed_hash.clone();
    }
    write_profile_state(&profile, store, &managed_hash)?;
    save_store(store)?;
    verify_models_endpoint(store).await?;
    store.lifecycle = UnifiedGatewayLifecycle::Active;
    store.updated_at = now_ms();
    save_store(store)?;
    push_event(
        "info",
        "gateway_active",
        "统一模型网关已激活，官方 auth.json 未改写",
    );
    Ok(())
}

async fn restore_on_failure(store: &UnifiedGatewayStore) -> Result<(), String> {
    stop_runtime().await;
    let profile = profile_dir();
    match restore_official_mode(&profile, store, true) {
        Ok(()) => Ok(()),
        Err(error) if error == "CONFIG_CONFLICT" => Ok(()),
        Err(error) => Err(error),
    }
}

pub async fn disable(force_backup: bool) -> Result<UnifiedGatewayStateView, String> {
    let mut store = load_store()?;
    let profile = profile_dir();
    stop_runtime().await;
    match restore_official_mode(&profile, &store, force_backup) {
        Ok(()) => {
            store.lifecycle = UnifiedGatewayLifecycle::Disabled;
            store.last_error = None;
            store.last_error_code = None;
            store.updated_at = now_ms();
            save_store(&store)?;
            push_event("info", "gateway_disabled", "已恢复官方 Codex 配置");
        }
        Err(error) if error == "CONFIG_CONFLICT" => {
            store.lifecycle = UnifiedGatewayLifecycle::RecoveryRequired;
            store.last_error = Some("用户修改了 config.toml，停用时不会覆盖当前文件。".to_string());
            store.last_error_code = Some("config_conflict".to_string());
            save_store(&store)?;
            return Err("CONFIG_CONFLICT".to_string());
        }
        Err(error) => return Err(error),
    }
    get_state().await
}

pub async fn resolve_conflict(restore_backup: bool) -> Result<UnifiedGatewayStateView, String> {
    disable(restore_backup).await
}

pub async fn select_grok_accounts(
    account_ids: Vec<String>,
) -> Result<UnifiedGatewayStateView, String> {
    let mut store = load_store()?;
    let known = grok_account::list_accounts_checked()?;
    let mut refs = Vec::new();
    for (index, account_id) in account_ids.into_iter().enumerate() {
        let account = known
            .iter()
            .find(|item| item.id == account_id)
            .ok_or_else(|| format!("Grok 账号不存在: {account_id}"))?;
        if account.auth_mode.as_str() != "oauth" {
            return Err("只有 OAuth 账号可以加入 Grok (OAuth) 池".to_string());
        }
        refs.push(UnifiedCredentialRef {
            id: format!("grok-ref-{account_id}"),
            kind: UnifiedCredentialKind::GrokOauth,
            account_id: Some(account_id),
            enabled: true,
            priority: index as i32,
            weight: 1,
            label: Some(account.email.clone()),
            ..UnifiedCredentialRef::default()
        });
    }
    store
        .credential_refs
        .retain(|item| item.kind != UnifiedCredentialKind::GrokOauth);
    store.credential_refs.extend(refs);
    if let Some(provider) = store
        .providers
        .iter_mut()
        .find(|item| item.id == GROK_OAUTH_PROVIDER_ID)
    {
        provider.enabled = store
            .credential_refs
            .iter()
            .any(|item| item.kind == UnifiedCredentialKind::GrokOauth && item.enabled);
        provider.credential_ref_ids = store
            .credential_refs
            .iter()
            .filter(|item| item.kind == UnifiedCredentialKind::GrokOauth)
            .map(|item| item.id.clone())
            .collect();
    }
    for model in store
        .models
        .iter_mut()
        .filter(|item| item.provider_id == GROK_OAUTH_PROVIDER_ID)
    {
        model.enabled = store
            .providers
            .iter()
            .any(|item| item.id == GROK_OAUTH_PROVIDER_ID && item.enabled);
    }
    store.updated_at = now_ms();
    save_store(&store)?;
    if store.lifecycle == UnifiedGatewayLifecycle::Active {
        write_catalog(&profile_dir(), &store)?;
        start_runtime(&store).await?;
    }
    get_state().await
}

pub async fn import_local_grok(path: Option<String>) -> Result<UnifiedGatewayStateView, String> {
    let imported = if let Some(path) = path {
        grok_account::import_from_path_with_binding(&path)?
    } else {
        grok_account::import_from_local()?
    };
    let mut ids = load_store()?
        .credential_refs
        .iter()
        .filter_map(|item| item.account_id.clone())
        .collect::<Vec<_>>();
    for account in imported {
        if !ids.iter().any(|id| id == &account.id) {
            ids.push(account.id);
        }
    }
    select_grok_accounts(ids).await
}

pub async fn upsert_api_provider(
    draft: UnifiedApiProviderDraft,
) -> Result<UnifiedGatewayStateView, String> {
    if draft.display_name.trim().is_empty() {
        return Err("Provider 显示名称不能为空".to_string());
    }
    let wire_api = normalize_provider_wire_api(draft.wire_api.as_deref());
    let base_url = normalize_provider_base_url(&draft.base_url)?;
    let mut store = load_store()?;
    let provider_id = draft
        .provider_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if draft.provider_type == UnifiedProviderType::XaiApi {
                XAI_API_PROVIDER_ID.to_string()
            } else {
                stable_provider_id(&draft.display_name, &draft.base_url)
            }
        });
    let cred_id = format!("cred-{provider_id}");
    let previous_secret_id = store
        .credential_refs
        .iter()
        .find(|item| item.id == cred_id)
        .and_then(|item| item.secret_id.clone());
    let secret_id = if draft.api_key.trim().is_empty() {
        previous_secret_id
            .clone()
            .ok_or_else(|| "请填写 API Key，统一网关不会从旧 Router 读取密钥。".to_string())?
    } else {
        let secret_id = format!("secret_{}", random_token(12));
        credential_broker::write_api_key_secret(&secret_id, draft.api_key.trim())?;
        secret_id
    };
    store.credential_refs.retain(|item| item.id != cred_id);
    store.credential_refs.push(UnifiedCredentialRef {
        id: cred_id.clone(),
        kind: UnifiedCredentialKind::ApiKey,
        secret_id: Some(secret_id.clone()),
        enabled: true,
        label: Some(draft.display_name.clone()),
        ..UnifiedCredentialRef::default()
    });
    store.providers.retain(|item| item.id != provider_id);
    store.providers.push(UnifiedModelProvider {
        id: provider_id.clone(),
        provider_type: draft.provider_type,
        display_name: draft.display_name,
        enabled: true,
        credential_ref_ids: vec![cred_id],
        base_url: Some(base_url),
        wire_api: Some(wire_api),
        ..UnifiedModelProvider::default()
    });
    let official = official_models()
        .into_iter()
        .map(|model| model.id)
        .collect::<HashSet<_>>();
    store
        .models
        .retain(|model| model.provider_id != provider_id);
    for model in draft.models {
        let trimmed = model.trim();
        if trimmed.is_empty() {
            continue;
        }
        store.models.push(namespaced_model(
            UnifiedModel {
                id: format!("{provider_id}/{trimmed}"),
                display_name: trimmed.to_string(),
                provider_id: provider_id.clone(),
                upstream_model: trimmed.to_string(),
                route: provider_id.clone(),
                enabled: true,
                capabilities: infer_model_capabilities(trimmed),
                context_window: infer_context_window(trimmed),
                reasoning_efforts: infer_reasoning_efforts(trimmed),
                availability: "available".to_string(),
            },
            &official,
        ));
    }
    store.updated_at = now_ms();
    save_store(&store)?;
    if let Some(previous_secret_id) = previous_secret_id {
        if previous_secret_id != secret_id
            && !store
                .credential_refs
                .iter()
                .any(|item| item.secret_id.as_deref() == Some(previous_secret_id.as_str()))
        {
            let _ = credential_broker::delete_api_key_secret(&previous_secret_id);
        }
    }
    if store.lifecycle == UnifiedGatewayLifecycle::Active {
        write_catalog(&profile_dir(), &store)?;
        start_runtime(&store).await?;
    }
    get_state().await
}

fn infer_model_capabilities(model_id: &str) -> UnifiedModelCapabilities {
    let lower = model_id.to_ascii_lowercase();
    let is_vision = lower.contains("vision")
        || lower.contains("vl")
        || lower.contains("omni")
        || lower.contains("4o")
        || lower.contains("claude-3")
        || lower.contains("gemini")
        || lower.contains("mimo-v2.5");
    UnifiedModelCapabilities {
        text: true,
        streaming: true,
        tools: true, // 核心：默认开启工具调用
        vision: is_vision,
        search: false,
    }
}

fn infer_context_window(model_id: &str) -> Option<i64> {
    let lower = model_id.to_ascii_lowercase();
    if lower.contains("1m") || lower.contains("1000k") || lower.contains("kimi-k3") {
        Some(1_000_000)
    } else if lower.contains("256k") {
        Some(262_144)
    } else if lower.contains("200k") || lower.contains("claude") {
        Some(200_000)
    } else if lower.contains("128k")
        || lower.contains("deepseek")
        || lower.contains("qwen")
        || lower.contains("glm")
    {
        Some(131_072)
    } else if lower.contains("32k") {
        Some(32_768)
    } else {
        Some(131_072)
    }
}

fn infer_reasoning_efforts(model_id: &str) -> Vec<String> {
    let lower = model_id.to_ascii_lowercase();
    if lower.contains("reasoner")
        || lower.contains("r1")
        || lower.contains("o1")
        || lower.contains("o3")
        || lower.contains("thinking")
        || lower.contains("grok")
    {
        vec!["low".to_string(), "medium".to_string(), "high".to_string()]
    } else {
        Vec::new()
    }
}

pub async fn test_api_provider(
    base_url: String,
    api_key: String,
    wire_api: String,
) -> Result<Vec<String>, String> {
    let base_url = normalize_provider_base_url(&base_url)?;
    let _ = normalize_provider_wire_api(Some(&wire_api));
    let mut url = Url::parse(&base_url).map_err(|error| format!("Base URL 无效: {error}"))?;
    let path = url.path().trim_end_matches('/');
    let endpoint = if path.is_empty() || path == "/" {
        "/v1/models".to_string()
    } else if path.ends_with("/v1") {
        format!("{path}/models")
    } else {
        format!("{path}/v1/models")
    };
    url.set_path(&endpoint);

    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败: {error}"))?;

    let mut req = client.get(url.as_str());
    if !api_key.trim().is_empty() {
        req = req.header("Authorization", format!("Bearer {}", api_key.trim()));
    }
    let resp = req
        .send()
        .await
        .map_err(|error| format!("连接 Provider 失败: {error}"))?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Provider 返回 HTTP {status}: {body}"));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|error| format!("解析模型列表响应失败: {error}"))?;
    let mut model_ids = Vec::new();
    if let Some(data) = body.get("data").and_then(Value::as_array) {
        for item in data {
            if let Some(id) = item.get("id").and_then(Value::as_str) {
                model_ids.push(id.to_string());
            }
        }
    } else if let Some(models) = body.get("models").and_then(Value::as_array) {
        for item in models {
            if let Some(id) = item
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| item.get("name").and_then(Value::as_str))
            {
                model_ids.push(id.to_string());
            }
        }
    }
    model_ids.sort();
    model_ids.dedup();
    Ok(model_ids)
}

fn normalize_provider_wire_api(value: Option<&str>) -> String {
    if value
        .map(|value| value.to_ascii_lowercase().contains("chat"))
        .unwrap_or(false)
    {
        "chat_completions".to_string()
    } else {
        "responses".to_string()
    }
}

fn normalize_provider_base_url(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    let url = Url::parse(value).map_err(|error| format!("Base URL 无效: {error}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Base URL 必须是带主机名的 http(s) 地址".to_string());
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err("Base URL 不应包含 query 或 fragment".to_string());
    }
    Ok(value.to_string())
}

pub async fn delete_api_provider(provider_id: String) -> Result<UnifiedGatewayStateView, String> {
    let provider_id = provider_id.trim().to_string();
    if provider_id.is_empty()
        || provider_id == OFFICIAL_PROVIDER_ID
        || provider_id == GROK_OAUTH_PROVIDER_ID
    {
        return Err("官方 Provider 和 Grok OAuth Provider 不能删除".to_string());
    }
    let mut store = load_store()?;
    let Some(provider) = store.providers.iter().find(|item| item.id == provider_id) else {
        return Err(format!("Provider 不存在: {provider_id}"));
    };
    let credential_ids = provider.credential_ref_ids.clone();
    let secret_ids = store
        .credential_refs
        .iter()
        .filter(|item| credential_ids.iter().any(|id| id == &item.id))
        .filter_map(|item| item.secret_id.clone())
        .collect::<Vec<_>>();
    store.providers.retain(|item| item.id != provider_id);
    store.models.retain(|item| item.provider_id != provider_id);
    store
        .credential_refs
        .retain(|item| !credential_ids.iter().any(|id| id == &item.id));
    store.updated_at = now_ms();
    save_store(&store)?;
    for secret_id in secret_ids {
        credential_broker::delete_api_key_secret(&secret_id)?;
    }
    if store.lifecycle == UnifiedGatewayLifecycle::Active {
        write_catalog(&profile_dir(), &store)?;
        start_runtime(&store).await?;
    }
    get_state().await
}

fn stable_provider_id(display_name: &str, base_url: &str) -> String {
    let seed = if display_name.trim().is_empty() {
        base_url
    } else {
        display_name
    };
    let slug = seed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        "openai-compatible".to_string()
    } else {
        format!("openai-compat-{slug}")
    }
}

pub async fn set_model_enabled(
    model_id: String,
    enabled: bool,
) -> Result<UnifiedGatewayStateView, String> {
    let mut store = load_store()?;
    let Some(model) = store.models.iter_mut().find(|item| item.id == model_id) else {
        return Err(format!("模型不存在: {model_id}"));
    };
    if model.provider_id == OFFICIAL_PROVIDER_ID && !enabled {
        return Err("官方 Codex 模型必须保留在目录中".to_string());
    }
    model.enabled = enabled;
    store.updated_at = now_ms();
    save_store(&store)?;
    if store.lifecycle == UnifiedGatewayLifecycle::Active {
        write_catalog(&profile_dir(), &store)?;
        start_runtime(&store).await?;
    }
    get_state().await
}

pub async fn set_routing_policy(
    policy: UnifiedRoutingPolicy,
) -> Result<UnifiedGatewayStateView, String> {
    let mut store = load_store()?;
    store.routing_policy = policy;
    store.updated_at = now_ms();
    save_store(&store)?;
    if store.lifecycle == UnifiedGatewayLifecycle::Active {
        start_runtime(&store).await?;
    }
    get_state().await
}

pub async fn migrate_from_router() -> Result<UnifiedGatewayStateView, String> {
    let preview = router_preview();
    if !preview.router_installed {
        return Err("未检测到旧 Codex Router 安装".to_string());
    }
    if preview.router_configured || preview.router_running {
        if let Err(error) = codex_router::disable() {
            return Err(format!("停用旧 Codex Router 失败: {error}"));
        }
    }
    if router_active() {
        return Err(
            "旧 Codex Router 仍在接管当前 Codex Profile，无法迁移。请先停用 Router。".to_string(),
        );
    }
    let mut ids = preview.matched_grok_account_ids;
    if ids.is_empty() && preview.importable_local_grok {
        if let Ok(imported) = grok_account::import_from_local() {
            ids.extend(imported.into_iter().map(|item| item.id));
        }
    }
    if !ids.is_empty() {
        select_grok_accounts(ids).await?;
    }
    enable().await
}

pub async fn restore_on_startup() {
    let Ok(mut store) = load_store() else {
        return;
    };
    if store.lifecycle != UnifiedGatewayLifecycle::Active {
        return;
    }
    let profile = profile_dir();
    if legacy_root_openai_config_is_owned(&profile, &store) {
        match apply_managed_config(&profile, &store) {
            Ok(managed_hash) => {
                if let Some(backup) = store.backup.as_mut() {
                    backup.managed_config_hash = managed_hash.clone();
                }
                if let Err(error) = write_profile_state(&profile, &store, &managed_hash) {
                    push_event("error", "startup_transport_upgrade_failed", &error);
                } else if let Err(error) = save_store(&store) {
                    push_event("error", "startup_transport_upgrade_failed", &error);
                } else {
                    push_event(
                        "info",
                        "startup_transport_upgraded",
                        "已迁移为保留 ChatGPT 鉴权的 HTTP/SSE 传输",
                    );
                }
            }
            Err(error) => push_event("error", "startup_transport_upgrade_failed", &error),
        }
    }
    if let Err(error) = write_catalog(&profile, &store) {
        push_event("warn", "startup_catalog_refresh_failed", &error);
    }
    if let Err(error) = start_runtime(&store).await {
        push_event("error", "startup_restore_failed", &error);
    }
}

fn legacy_root_openai_config_is_owned(profile: &Path, store: &UnifiedGatewayStore) -> bool {
    let Some(state) = read_profile_state(profile) else {
        return false;
    };
    if state.ownership_id != store.ownership_id
        || state.mode != "root-openai"
        || state.managed_provider != "openai"
        || state.managed_base_url != capability_base_url(store)
    {
        return false;
    }
    let Ok(content) = std::fs::read_to_string(config_path(profile)) else {
        return false;
    };
    let Ok(doc) = codex_config_format::read_codex_config_doc_from_str(&content) else {
        return false;
    };
    doc.get("openai_base_url")
        .and_then(|item| item.as_str())
        .is_some_and(|value| value == capability_base_url(store))
}

pub async fn shutdown_for_exit() {
    stop_runtime().await;
}

fn sidecar_config_paths() -> Result<(PathBuf, PathBuf), String> {
    let dir = sidecar_dir()?;
    Ok((dir.join("config.json"), dir.join("manifest.json")))
}

fn sidecar_auth_dir() -> Result<PathBuf, String> {
    let path = sidecar_dir()?.join("auths");
    std::fs::create_dir_all(&path)
        .map_err(|error| format!("创建统一网关 auth 目录失败: {error}"))?;
    Ok(path)
}

fn write_sidecar_files(store: &UnifiedGatewayStore, socket_path: &Path) -> Result<(), String> {
    let (config_path, manifest_path) = sidecar_config_paths()?;
    let auth_dir = sidecar_auth_dir()?;
    let config = json!({
        "host": if store.access_scope == UnifiedAccessScope::Lan { "0.0.0.0" } else { "127.0.0.1" },
        "port": store.port,
        "auth-dir": auth_dir.to_string_lossy(),
        "debug": false,
        "api-keys": [],
        "request-log": false,
        "logging-to-file": false,
        "commercial-mode": true,
        "ws-auth": true,
        "disable-auth-auto-refresh": true,
    });
    write_string_atomic(
        &config_path,
        &serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?,
    )?;
    let manifest = json!({
        "apiKeys": [],
        "accounts": [],
        "modelIds": store.models.iter().filter(|item| item.enabled).map(|item| item.id.clone()).collect::<Vec<_>>(),
        "unifiedGateway": sidecar_manifest(store, socket_path),
    });
    write_string_atomic(
        &manifest_path,
        &serde_json::to_string_pretty(&manifest).map_err(|error| error.to_string())?,
    )?;
    Ok(())
}

fn sidecar_manifest(store: &UnifiedGatewayStore, socket_path: &Path) -> Value {
    json!({
        "enabled": true,
        "protocolVersion": UNIFIED_GATEWAY_PROTOCOL_VERSION,
        "capabilityToken": store.capability_token,
        "lanMode": store.access_scope == UnifiedAccessScope::Lan,
        "officialPassthrough": true,
        "officialUpstream": "https://chatgpt.com/backend-api/codex",
        "brokerSocket": socket_path.display().to_string(),
        "routingPolicy": store.routing_policy,
        "routes": store.models.iter().filter(|item| item.enabled).map(|model| json!({
            "modelId": model.id,
            "providerId": model.provider_id,
            "route": model.route,
            "upstreamModel": model.upstream_model,
            "capabilities": model.capabilities,
            "reasoningEfforts": model.reasoning_efforts,
        })).collect::<Vec<_>>(),
        "grokPool": store.credential_refs.iter().filter(|item| item.kind == UnifiedCredentialKind::GrokOauth && item.enabled).map(|item| json!({
            "accountId": item.account_id,
            "priority": item.priority,
            "weight": item.weight,
            "backupOnly": item.backup_only,
            "minRemainingPercent": item.min_remaining_percent,
            "allowedModels": item.allowed_models,
        })).collect::<Vec<_>>(),
        "providers": store.providers.iter().filter(|item| item.enabled).map(|item| json!({
            "id": item.id,
            "type": item.provider_type,
            "baseUrl": item.base_url,
            "wireApi": item.wire_api,
            "credentialRefIds": item.credential_ref_ids,
        })).collect::<Vec<_>>(),
    })
}

async fn start_runtime(store: &UnifiedGatewayStore) -> Result<(), String> {
    stop_runtime().await;
    let secrets = BrokerLaunchSecrets::generate();
    write_sidecar_files(store, &credential_broker::default_socket_path()?)?;
    credential_broker::start_broker(secrets.clone(), 0).await?;
    let binary = cliproxy_binary_path()?;
    let (config_path, manifest_path) = sidecar_config_paths()?;
    let mut command = TokioCommand::new(&binary);
    #[cfg(target_os = "macos")]
    {
        command.env_remove("__CFBundleIdentifier");
        command.env_remove("XPC_SERVICE_NAME");
    }
    command
        .arg("--config")
        .arg(&config_path)
        .arg("--manifest")
        .arg(&manifest_path)
        .arg("--parent-pid")
        .arg(std::process::id().to_string())
        .current_dir(sidecar_dir()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("启动统一网关 sidecar 失败: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&secrets.encode_handshake())
            .await
            .map_err(|error| format!("写入 sidecar 握手失败: {error}"))?;
    }
    let last_error = Arc::new(Mutex::new(None::<String>));
    if let Some(stdout) = child.stdout.take() {
        let last_error = Arc::clone(&last_error);
        tauri::async_runtime::spawn(async move {
            drain_sidecar_stream(stdout, last_error).await;
        });
    }
    if let Some(stderr) = child.stderr.take() {
        let last_error = Arc::clone(&last_error);
        tauri::async_runtime::spawn(async move {
            drain_sidecar_stream(stderr, last_error).await;
        });
    }
    let pid = child.id().unwrap_or(0);
    if pid != 0 {
        credential_broker::set_expected_child_pid(pid).await;
    }
    let ready = wait_sidecar_ready(store.port, &mut child, &last_error).await;
    if let Err(error) = &ready {
        let _ = child.kill().await;
        let mut runtime = runtime().lock().await;
        runtime.running = false;
        runtime.child_pid = None;
        runtime.last_error = Some(error.clone());
        return Err(error.clone());
    }
    let mut runtime = runtime().lock().await;
    runtime.running = true;
    runtime.child_pid = Some(pid);
    runtime.last_error = None;
    drop(runtime);
    tauri::async_runtime::spawn(async move {
        watch_child(child).await;
    });
    Ok(())
}

async fn watch_child(mut child: Child) {
    let _ = child.wait().await;
    let mut runtime = runtime().lock().await;
    runtime.running = false;
    runtime.child_pid = None;
}

async fn stop_runtime() {
    let port = load_store().ok().map(|store| store.port);
    credential_broker::stop_broker().await;
    {
        let mut runtime = runtime().lock().await;
        runtime.running = false;
        runtime.child_pid = None;
    }
    if let Some(port) = port {
        let _ = crate::modules::process::kill_port_processes(port);
    }
}

async fn drain_sidecar_stream<R>(stream: R, last_error: Arc<Mutex<Option<String>>>)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        push_event("info", "sidecar", trimmed);
        if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
            if value.get("type").and_then(Value::as_str) == Some("error") {
                if let Some(message) = value.get("message").and_then(Value::as_str) {
                    if let Ok(mut slot) = last_error.lock() {
                        *slot = Some(message.to_string());
                    }
                }
            }
        }
    }
}

fn sidecar_startup_error(last_error: &Arc<Mutex<Option<String>>>, fallback: String) -> String {
    last_error
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .filter(|message| !message.trim().is_empty())
        .unwrap_or(fallback)
}

async fn wait_sidecar_ready(
    port: u16,
    child: &mut Child,
    last_error: &Arc<Mutex<Option<String>>>,
) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/healthz");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(400))
        .build()
        .map_err(|error| error.to_string())?;
    for _ in 0..80 {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(sidecar_startup_error(
                last_error,
                format!("统一网关 sidecar 已退出 ({status})"),
            ));
        }
        if client.get(&url).send().await.is_ok() {
            return Ok(());
        }
        if TcpReady::connect(port) {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    Err(sidecar_startup_error(
        last_error,
        format!("统一网关 sidecar 未在端口 {port} 就绪"),
    ))
}

struct TcpReady;
impl TcpReady {
    fn connect(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
            std::time::Duration::from_millis(150),
        )
        .is_ok()
    }
}

async fn verify_models_endpoint(store: &UnifiedGatewayStore) -> Result<(), String> {
    let url = format!("{}/models", capability_base_url(store));
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("验证模型目录失败: {error}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "GET /v1/models 返回 {}",
            response.status().as_u16()
        ));
    }
    Ok(())
}

fn cliproxy_binary_path() -> Result<PathBuf, String> {
    crate::modules::codex_local_access::cliproxy_sidecar_binary_path()
}

pub fn public_store_for_export(store: &UnifiedGatewayStore) -> UnifiedGatewayStore {
    let mut exported = store.clone();
    for item in &mut exported.credential_refs {
        if item.secret_id.is_some() {
            item.secret_id = Some("stored".to_string());
        }
    }
    exported.capability_token.clear();
    exported
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::test_support::env_lock;
    use std::fs;

    fn isolated_env() -> (
        std::sync::MutexGuard<'static, ()>,
        tempfile_guard::Guard,
        PathBuf,
    ) {
        let lock = env_lock().lock().unwrap_or_else(|error| error.into_inner());
        let dir = std::env::temp_dir().join(format!("ugw-test-{}", random_token(8)));
        fs::create_dir_all(&dir).unwrap();
        std::env::set_var("COCKPIT_TOOLS_TEST_DATA_DIR", &dir);
        std::env::set_var("COCKPIT_TOOLS_DATA_DIR", &dir);
        let home = dir.join("home");
        fs::create_dir_all(home.join(".codex")).unwrap();
        std::env::set_var("HOME", &home);
        std::env::set_var("CODEX_HOME", home.join(".codex"));
        (
            lock,
            tempfile_guard::Guard { dir: dir.clone() },
            home.join(".codex"),
        )
    }

    mod tempfile_guard {
        pub struct Guard {
            pub dir: std::path::PathBuf,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.dir);
            }
        }
    }

    #[test]
    fn migrates_empty_store_with_official_and_grok_models() {
        let _env = isolated_env();
        let store = load_store().unwrap();
        assert_eq!(store.version, UNIFIED_GATEWAY_STORE_VERSION);
        assert!(store
            .providers
            .iter()
            .any(|item| item.id == OFFICIAL_PROVIDER_ID));
        assert!(store
            .models
            .iter()
            .any(|item| item.provider_id == GROK_OAUTH_PROVIDER_ID));
        assert!(store
            .models
            .iter()
            .any(|item| item.provider_id == OFFICIAL_PROVIDER_ID));
    }

    #[test]
    fn namespaces_external_model_when_official_id_collides() {
        let official = HashSet::from(["gpt-5.4".to_string()]);
        let model = namespaced_model(
            UnifiedModel {
                id: "gpt-5.4".to_string(),
                provider_id: "xai-api".to_string(),
                upstream_model: "gpt-5.4".to_string(),
                ..UnifiedModel::default()
            },
            &official,
        );
        assert_eq!(model.id, "xai-api/gpt-5.4");
    }

    #[test]
    fn catalog_never_lists_disabled_external_models() {
        let _env = isolated_env();
        let mut store = load_store().unwrap();
        for model in store
            .models
            .iter_mut()
            .filter(|item| item.provider_id == GROK_OAUTH_PROVIDER_ID)
        {
            model.enabled = false;
        }
        let catalog = build_catalog_json(&store).unwrap();
        assert!(!catalog.contains("grok-4.5"));
    }

    #[test]
    fn catalog_prefers_websockets_only_for_official_models() {
        let _env = isolated_env();
        let mut store = load_store().unwrap();
        for model in store.models.iter_mut() {
            if model.provider_id == GROK_OAUTH_PROVIDER_ID {
                model.enabled = true;
            }
        }
        let catalog = build_catalog_json(&store).unwrap();
        let parsed: Value = serde_json::from_str(&catalog).unwrap();
        let models = parsed.get("models").and_then(Value::as_array).unwrap();
        let mut saw_official = false;
        let mut saw_grok = false;
        for model in models {
            let id = model
                .get("id")
                .and_then(Value::as_str)
                .or_else(|| model.get("slug").and_then(Value::as_str))
                .unwrap_or_default();
            let prefer = model
                .get("prefer_websockets")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if id.starts_with("gpt-") {
                assert!(prefer, "{id} should prefer websockets");
                saw_official = true;
            }
            if id.starts_with("grok-oauth/") {
                assert!(!prefer, "{id} should use HTTP responses");
                saw_grok = true;
            }
        }
        assert!(saw_official && saw_grok, "catalog={catalog}");
    }

    #[test]
    fn sidecar_config_writes_auth_dir() {
        let _env = isolated_env();
        let store = load_store().unwrap();
        write_sidecar_files(&store, Path::new("/tmp/credential-broker.sock")).unwrap();
        let config = fs::read_to_string(sidecar_dir().unwrap().join("config.json")).unwrap();
        assert!(
            config.contains("auth-dir"),
            "sidecar config missing auth-dir: {config}"
        );
        assert!(sidecar_dir().unwrap().join("auths").is_dir());
    }

    #[test]
    fn apply_and_remove_managed_fields_preserves_unrelated_keys() {
        let (_lock, _guard, profile) = isolated_env();
        fs::write(
            profile.join("config.toml"),
            "model = \"gpt-5.4\"\nproject_doc_max_bytes = 12\n",
        )
        .unwrap();
        fs::write(
            profile.join("auth.json"),
            "{\"tokens\":{\"access_token\":\"keep-me\"}}",
        )
        .unwrap();
        let mut store = load_store().unwrap();
        store.capability_token = "tokentoken1234567890abcd".to_string();
        store.port = 1457;
        apply_managed_config(&profile, &store).unwrap();
        let auth = fs::read_to_string(profile.join("auth.json")).unwrap();
        assert!(auth.contains("keep-me"));
        let config = fs::read_to_string(profile.join("config.toml")).unwrap();
        assert!(config.contains("model_provider = \"cockpit-unified-gateway\""));
        assert!(config.contains("[model_providers.cockpit-unified-gateway]"));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains("supports_websockets = false"));
        assert!(!config.contains("openai_base_url"));
        assert!(config.contains("project_doc_max_bytes = 12"));
        assert!(
            config.contains("/_cockpit-ugw/"),
            "managed config missing capability path: {config}"
        );
        let catalog = catalog_path(&profile);
        assert!(
            config.contains(&catalog.to_string_lossy().to_string()),
            "model_catalog_json must be an absolute path: {config}"
        );
        let mut doc = codex_config_format::read_codex_config_doc_from_str(&config).unwrap();
        remove_managed_fields(&mut doc);
        let restored = doc.to_string();
        assert!(!restored.contains("openai_base_url"));
        assert!(!restored.contains("model_catalog_json"));
        assert!(!restored.contains("cockpit-unified-gateway"));
        assert!(restored.contains("project_doc_max_bytes"));
    }

    #[test]
    fn apply_managed_config_strips_reserved_openai_provider_override() {
        let (_lock, _guard, profile) = isolated_env();
        fs::write(
            profile.join("config.toml"),
            "model = \"gpt-5.4\"\n\n[model_providers.openai]\nrequires_openai_auth = true\n",
        )
        .unwrap();
        let mut store = load_store().unwrap();
        store.capability_token = "tokentoken1234567890abcd".to_string();
        apply_managed_config(&profile, &store).unwrap();
        let config = fs::read_to_string(profile.join("config.toml")).unwrap();
        assert!(!config.contains("[model_providers.openai]"));
        assert!(config.contains("[model_providers.cockpit-unified-gateway]"));
        assert!(config.contains("requires_openai_auth = true"));
        assert!(config.contains("supports_websockets = false"));
    }

    #[test]
    fn legacy_root_openai_config_is_recognized_for_startup_upgrade() {
        let (_lock, _guard, profile) = isolated_env();
        let mut store = load_store().unwrap();
        store.capability_token = "tokentoken1234567890abcd".to_string();
        let base_url = capability_base_url(&store);
        fs::write(
            profile.join("config.toml"),
            format!("model = \"grok-oauth/grok-4.5\"\nopenai_base_url = \"{base_url}\"\n"),
        )
        .unwrap();
        let legacy_state = UnifiedGatewayProfileState {
            version: 1,
            ownership_id: store.ownership_id.clone(),
            mode: "root-openai".to_string(),
            managed_provider: "openai".to_string(),
            managed_base_url: base_url,
            catalog_file: catalog_path(&profile).to_string_lossy().to_string(),
            original_config_hash: String::new(),
            managed_config_hash: String::new(),
            updated_at: now_ms(),
        };
        fs::write(
            profile_state_path(&profile),
            serde_json::to_string(&legacy_state).unwrap(),
        )
        .unwrap();
        assert!(legacy_root_openai_config_is_owned(&profile, &store));
    }

    #[test]
    fn conflict_detected_when_user_edits_config() {
        let (_lock, _guard, profile) = isolated_env();
        let mut store = load_store().unwrap();
        store.capability_token = "abc".to_string();
        backup_profile(&profile, &mut store).unwrap();
        apply_managed_config(&profile, &store).unwrap();
        if let Some(backup) = store.backup.as_mut() {
            backup.managed_config_hash = "managed".to_string();
        }
        fs::write(profile.join("config.toml"), "model = \"user-changed\"\n").unwrap();
        let conflict = inspect_conflict(&profile, &store);
        assert!(conflict.present);
        let err = restore_official_mode(&profile, &store, false).unwrap_err();
        assert_eq!(err, "CONFIG_CONFLICT");
        assert_eq!(
            fs::read_to_string(profile.join("config.toml")).unwrap(),
            "model = \"user-changed\"\n"
        );
    }

    #[test]
    fn restore_strips_catalog_key_even_when_backup_still_points_at_it() {
        let (_lock, _guard, profile) = isolated_env();
        let catalog = catalog_path(&profile);
        fs::write(
            profile.join("config.toml"),
            format!(
                "model = \"gpt-5.6-luna\"\nproject_doc_max_bytes = 12\nmodel_catalog_json = \"{}\"\nopenai_base_url = \"http://127.0.0.1:1457/_cockpit-ugw/tokentoken1234567890abcd/v1\"\n",
                catalog.display()
            ),
        )
        .unwrap();
        fs::write(&catalog, "{\"models\":[]}\n").unwrap();
        let mut store = load_store().unwrap();
        backup_profile(&profile, &mut store).unwrap();
        let backup_id = store.backup.as_ref().unwrap().backup_id.clone();
        let backup_file = backup_root().unwrap().join(backup_id).join("config.toml");
        fs::write(
            &backup_file,
            format!(
                "model = \"gpt-5.6-luna\"\nproject_doc_max_bytes = 12\nmodel_catalog_json = \"{}\"\n",
                catalog.display()
            ),
        )
        .unwrap();
        restore_official_mode(&profile, &store, true).unwrap();
        let restored = fs::read_to_string(profile.join("config.toml")).unwrap();
        assert!(
            !restored.contains("model_catalog_json"),
            "dangling catalog key left behind: {restored}"
        );
        assert!(!restored.contains("openai_base_url"), "{restored}");
        assert!(restored.contains("project_doc_max_bytes"), "{restored}");
        assert!(!catalog.exists(), "catalog file should be removed");
    }

    #[test]
    fn backup_profile_strips_managed_keys() {
        let (_lock, _guard, profile) = isolated_env();
        fs::write(
            profile.join("config.toml"),
            "model = \"gpt-5.6-luna\"\nmodel_catalog_json = \"/tmp/missing.json\"\nopenai_base_url = \"http://127.0.0.1:1457/_cockpit-ugw/tok/v1\"\nkeep_me = true\n",
        )
        .unwrap();
        let mut store = load_store().unwrap();
        backup_profile(&profile, &mut store).unwrap();
        let backup_id = store.backup.as_ref().unwrap().backup_id.clone();
        let backup =
            fs::read_to_string(backup_root().unwrap().join(backup_id).join("config.toml")).unwrap();
        assert!(!backup.contains("model_catalog_json"), "{backup}");
        assert!(!backup.contains("openai_base_url"), "{backup}");
        assert!(backup.contains("keep_me"), "{backup}");
    }

    #[test]
    fn sidecar_manifest_uses_account_refs_and_keeps_official_routes() {
        let _env = isolated_env();
        let mut store = load_store().unwrap();
        store.credential_refs.push(UnifiedCredentialRef {
            id: "grok-ref-acc-1".to_string(),
            kind: UnifiedCredentialKind::GrokOauth,
            account_id: Some("acc-1".to_string()),
            enabled: true,
            ..UnifiedCredentialRef::default()
        });
        for model in store
            .models
            .iter_mut()
            .filter(|item| item.provider_id == GROK_OAUTH_PROVIDER_ID)
        {
            model.enabled = true;
        }
        let manifest = sidecar_manifest(&store, Path::new("/tmp/credential-broker.sock"));
        let grok_pool = manifest
            .get("grokPool")
            .and_then(|item| item.as_array())
            .unwrap();
        assert_eq!(grok_pool[0]["accountId"], "acc-1");
        assert!(grok_pool[0].get("accessToken").is_none());
        assert!(grok_pool[0].get("refreshToken").is_none());
        let routes = manifest
            .get("routes")
            .and_then(|item| item.as_array())
            .unwrap();
        assert!(routes.iter().any(|route| {
            route.get("route").and_then(|item| item.as_str()) == Some("official")
        }));
        assert!(routes.iter().any(|route| {
            route.get("route").and_then(|item| item.as_str()) == Some("grok-oauth")
                && route.get("modelId").and_then(|item| item.as_str())
                    == Some("grok-oauth/grok-4.5")
        }));
        let encoded = serde_json::to_string(&manifest).unwrap();
        assert!(!encoded.contains("refresh_token"));
        assert!(!encoded.contains("sk-"));
    }

    #[test]
    fn public_export_strips_capability_and_secret_ids() {
        let mut store = UnifiedGatewayStore::default();
        store.capability_token = "super-secret".to_string();
        store.credential_refs.push(UnifiedCredentialRef {
            id: "c1".to_string(),
            secret_id: Some("secret_abc".to_string()),
            ..UnifiedCredentialRef::default()
        });
        let exported = public_store_for_export(&store);
        assert!(exported.capability_token.is_empty());
        assert_eq!(
            exported.credential_refs[0].secret_id.as_deref(),
            Some("stored")
        );
    }
}
