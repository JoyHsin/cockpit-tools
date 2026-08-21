use serde::{Deserialize, Serialize};

pub const UNIFIED_GATEWAY_STORE_VERSION: u32 = 1;
pub const UNIFIED_GATEWAY_STATE_FILE: &str = "unified-gateway-state.json";
pub const UNIFIED_GATEWAY_MODEL_CATALOG_FILE: &str = "cockpit-unified-gateway-model-catalog.json";
pub const UNIFIED_GATEWAY_ENDPOINT_MARKER: &str = "/_cockpit-ugw/";
pub const UNIFIED_GATEWAY_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedGatewayLifecycle {
    #[default]
    Disabled,
    Preparing,
    Configured,
    Verifying,
    Active,
    RecoveryRequired,
}

impl UnifiedGatewayLifecycle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Preparing => "preparing",
            Self::Configured => "configured",
            Self::Verifying => "verifying",
            Self::Active => "active",
            Self::RecoveryRequired => "recovery_required",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedProviderType {
    #[default]
    OfficialCodex,
    GrokOauth,
    XaiApi,
    OpenaiCompatible,
}

impl UnifiedProviderType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OfficialCodex => "official_codex",
            Self::GrokOauth => "grok_oauth",
            Self::XaiApi => "xai_api",
            Self::OpenaiCompatible => "openai_compatible",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum UnifiedCredentialKind {
    #[default]
    GrokOauth,
    ApiKey,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedRoutingMode {
    SingleAccount,
    Weighted,
    #[default]
    Priority,
    Backup,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum UnifiedAccessScope {
    #[default]
    Localhost,
    Lan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedModelCapabilities {
    #[serde(default = "default_true")]
    pub text: bool,
    #[serde(default = "default_true")]
    pub streaming: bool,
    #[serde(default = "default_true")]
    pub tools: bool,
    #[serde(default)]
    pub vision: bool,
    #[serde(default)]
    pub search: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedModelProvider {
    pub id: String,
    #[serde(rename = "type")]
    pub provider_type: UnifiedProviderType,
    pub display_name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub credential_ref_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wire_api: Option<String>,
    #[serde(default)]
    pub policy: UnifiedRoutingPolicy,
}

impl Default for UnifiedModelProvider {
    fn default() -> Self {
        Self {
            id: String::new(),
            provider_type: UnifiedProviderType::OfficialCodex,
            display_name: String::new(),
            enabled: true,
            credential_ref_ids: Vec::new(),
            base_url: None,
            wire_api: None,
            policy: UnifiedRoutingPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedModel {
    pub id: String,
    pub display_name: String,
    pub provider_id: String,
    pub upstream_model: String,
    #[serde(default)]
    pub route: String,
    #[serde(default)]
    pub capabilities: UnifiedModelCapabilities,
    #[serde(default)]
    pub reasoning_efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_availability")]
    pub availability: String,
}

fn default_availability() -> String {
    "available".to_string()
}

impl Default for UnifiedModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            display_name: String::new(),
            provider_id: String::new(),
            upstream_model: String::new(),
            route: String::new(),
            capabilities: UnifiedModelCapabilities::default(),
            reasoning_efforts: Vec::new(),
            context_window: None,
            enabled: true,
            availability: default_availability(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedCredentialRef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: UnifiedCredentialKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub weight: u32,
    #[serde(default)]
    pub backup_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_remaining_percent: Option<i32>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Default for UnifiedCredentialRef {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: UnifiedCredentialKind::GrokOauth,
            account_id: None,
            secret_id: None,
            enabled: true,
            priority: 0,
            weight: 1,
            backup_only: false,
            min_remaining_percent: None,
            allowed_models: Vec::new(),
            label: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedRoutingPolicy {
    #[serde(default)]
    pub mode: UnifiedRoutingMode,
    #[serde(default = "default_true")]
    pub session_affinity: bool,
    #[serde(default = "default_session_affinity_ttl_ms")]
    pub session_affinity_ttl_ms: i64,
}

fn default_session_affinity_ttl_ms() -> i64 {
    60 * 60 * 1000
}

impl Default for UnifiedRoutingPolicy {
    fn default() -> Self {
        Self {
            mode: UnifiedRoutingMode::Priority,
            session_affinity: true,
            session_affinity_ttl_ms: default_session_affinity_ttl_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedSharedSessionBinding {
    pub account_id: String,
    pub source_path: String,
    pub identity: String,
    pub credential_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<i64>,
    #[serde(default)]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayBackupRef {
    pub backup_id: String,
    pub original_config_hash: String,
    pub managed_config_hash: String,
    pub created_at: i64,
    pub profile_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayProfileState {
    pub version: u32,
    pub ownership_id: String,
    pub mode: String,
    pub managed_provider: String,
    pub managed_base_url: String,
    pub catalog_file: String,
    pub original_config_hash: String,
    pub managed_config_hash: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayStore {
    #[serde(default = "default_store_version")]
    pub version: u32,
    #[serde(default)]
    pub lifecycle: UnifiedGatewayLifecycle,
    #[serde(default)]
    pub ownership_id: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub access_scope: UnifiedAccessScope,
    #[serde(default = "default_client_host")]
    pub client_base_url_host: String,
    #[serde(default)]
    pub capability_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lan_api_key_secret_id: Option<String>,
    #[serde(default = "default_true")]
    pub official_auth_protected: bool,
    #[serde(default)]
    pub providers: Vec<UnifiedModelProvider>,
    #[serde(default)]
    pub models: Vec<UnifiedModel>,
    #[serde(default)]
    pub credential_refs: Vec<UnifiedCredentialRef>,
    #[serde(default)]
    pub routing_policy: UnifiedRoutingPolicy,
    #[serde(default)]
    pub shared_session_bindings: Vec<UnifiedSharedSessionBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup: Option<UnifiedGatewayBackupRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_code: Option<String>,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
}

fn default_store_version() -> u32 {
    UNIFIED_GATEWAY_STORE_VERSION
}

fn default_port() -> u16 {
    1457
}

fn default_client_host() -> String {
    "127.0.0.1".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayStatus {
    pub lifecycle: UnifiedGatewayLifecycle,
    pub running: bool,
    pub configured: bool,
    pub official_auth_protected: bool,
    pub ownership_matched: bool,
    pub port: u16,
    pub base_url: Option<String>,
    pub enabled_provider_count: usize,
    pub enabled_model_count: usize,
    pub grok_account_count: usize,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
    pub owner: String,
    pub conflict: bool,
    pub router_detected: bool,
    pub api_service_active: bool,
    pub supported_codex_range: String,
    pub threat_model_note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGrokAccountOption {
    pub account_id: String,
    pub email: String,
    pub auth_mode: String,
    pub eligible: bool,
    pub selected: bool,
    pub has_grok_code_access: bool,
    pub status: Option<String>,
    pub remaining_percent: Option<i32>,
    pub usage_updated_at: Option<i64>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ineligible_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGrokImportCandidate {
    pub source: String,
    pub path: String,
    pub identity: String,
    pub email: Option<String>,
    pub expires_at: Option<i64>,
    pub already_managed: bool,
    pub importable: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub provider_id: String,
    pub provider_type: UnifiedProviderType,
    pub upstream_model: String,
    pub enabled: bool,
    pub conflict: bool,
    pub capabilities: UnifiedModelCapabilities,
    pub availability: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayDiagnostics {
    pub lifecycle: UnifiedGatewayLifecycle,
    pub running: bool,
    pub official_auth_protected: bool,
    pub ownership_matched: bool,
    pub current_config_hash: Option<String>,
    pub managed_config_hash: Option<String>,
    pub original_config_hash: Option<String>,
    pub broker_listening: bool,
    pub sidecar_running: bool,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
    pub recent_events: Vec<UnifiedGatewayLogEvent>,
    pub provider_health: Vec<UnifiedProviderHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayLogEvent {
    pub at: i64,
    pub level: String,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedProviderHealth {
    pub provider_id: String,
    pub display_name: String,
    pub healthy: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayConflict {
    pub present: bool,
    pub current_hash: Option<String>,
    pub managed_hash: Option<String>,
    pub original_hash: Option<String>,
    pub current_preview: Option<String>,
    pub managed_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedRouterMigrationPreview {
    pub router_installed: bool,
    pub router_configured: bool,
    pub router_running: bool,
    pub providers: Vec<UnifiedRouterMigrationProvider>,
    pub matched_grok_account_ids: Vec<String>,
    pub importable_local_grok: bool,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedRouterMigrationProvider {
    pub id: String,
    pub display_name: String,
    pub kind: String,
    pub visible: bool,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedApiProviderDraft {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    pub display_name: String,
    pub provider_type: UnifiedProviderType,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub wire_api: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnifiedGatewayStateView {
    pub status: UnifiedGatewayStatus,
    pub providers: Vec<UnifiedModelProvider>,
    pub models: Vec<UnifiedGatewayCatalogEntry>,
    pub grok_accounts: Vec<UnifiedGrokAccountOption>,
    pub import_candidates: Vec<UnifiedGrokImportCandidate>,
    pub credential_refs: Vec<UnifiedCredentialRef>,
    pub routing_policy: UnifiedRoutingPolicy,
    pub diagnostics: UnifiedGatewayDiagnostics,
    pub conflict: UnifiedGatewayConflict,
    pub router_migration: UnifiedRouterMigrationPreview,
}
