use crate::models::codex::{
    CodexAccount, CodexApiModelMapping, CodexApiProviderMode, CodexAppSpeed, CodexAppSpeedConfig,
    CodexQuickConfig, CodexQuota, CodexTokens,
};
use crate::models::codex_local_access::{
    CodexLocalAccessAccountModelRule, CodexLocalAccessAccountWindowQuery,
    CodexLocalAccessAccountWindowStats, CodexLocalAccessAppendAccountsResult,
    CodexLocalAccessChatMessage, CodexLocalAccessChatResult, CodexLocalAccessClientBaseUrlHost,
    CodexLocalAccessCustomRoutingRule, CodexLocalAccessGatewayMode,
    CodexLocalAccessImageGenerationPolicy, CodexLocalAccessModelAlias,
    CodexLocalAccessModelPricing, CodexLocalAccessPortCleanupResult, CodexLocalAccessQuotaReserve,
    CodexLocalAccessRequestKind, CodexLocalAccessRoutingStrategy, CodexLocalAccessScope,
    CodexLocalAccessState, CodexLocalAccessTestFailure, CodexLocalAccessTestResult,
    CodexLocalAccessTimeoutPreset, CodexLocalAccessTimeouts, CodexLocalAccessUsageEventPage,
};
use crate::modules::{
    account, codex_account, codex_local_access, codex_oauth, codex_quota, codex_router,
    codex_session_visibility, codex_speed, codex_wakeup, codex_wakeup_scheduler, config,
    hermes_auth, logger, openclaw_auth, opencode_auth, process, unified_model_gateway,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tauri::Emitter;
use tauri_plugin_opener::OpenerExt;

#[tauri::command]
pub fn codex_router_get_status() -> crate::modules::codex_router::CodexRouterStatus {
    codex_router::get_status()
}

#[tauri::command]
pub fn codex_router_control_service(
    action: String,
) -> Result<crate::modules::codex_router::CodexRouterStatus, String> {
    codex_router::control_service(action.trim())
}

#[tauri::command]
pub fn codex_router_install() -> Result<crate::modules::codex_router::CodexRouterStatus, String> {
    codex_router::install()
}

#[tauri::command]
pub fn codex_router_update() -> Result<crate::modules::codex_router::CodexRouterStatus, String> {
    codex_router::update()
}

#[tauri::command]
pub fn codex_router_enable() -> Result<crate::modules::codex_router::CodexRouterStatus, String> {
    codex_router::enable()
}

#[tauri::command]
pub fn codex_router_disable() -> Result<crate::modules::codex_router::CodexRouterStatus, String> {
    codex_router::disable()
}

#[tauri::command]
pub fn codex_router_list_providers(
) -> Result<Vec<crate::modules::codex_router::CodexRouterProvider>, String> {
    codex_router::list_providers()
}

#[tauri::command]
pub fn codex_router_set_provider_enabled(
    provider_id: String,
    enabled: bool,
) -> Result<Vec<crate::modules::codex_router::CodexRouterProvider>, String> {
    codex_router::set_provider_enabled(&provider_id, enabled)
}

#[tauri::command]
pub fn codex_router_install_provider_cli(
    provider_id: String,
) -> Result<Vec<crate::modules::codex_router::CodexRouterProvider>, String> {
    codex_router::install_provider_cli(&provider_id)
}

#[tauri::command]
pub fn codex_router_login_provider(
    provider_id: String,
) -> Result<Vec<crate::modules::codex_router::CodexRouterProvider>, String> {
    codex_router::login_provider(&provider_id)
}

#[tauri::command]
pub fn codex_router_set_provider_key(
    provider_id: String,
    api_key: String,
) -> Result<Vec<crate::modules::codex_router::CodexRouterProvider>, String> {
    codex_router::set_provider_key(&provider_id, &api_key)
}

#[tauri::command]
pub fn codex_router_remove_provider_key(
    provider_id: String,
) -> Result<Vec<crate::modules::codex_router::CodexRouterProvider>, String> {
    codex_router::remove_provider_key(&provider_id)
}

#[tauri::command]
pub async fn unified_gateway_get_state(
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::get_state().await
}

#[tauri::command]
pub async fn unified_gateway_enable(
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::enable().await
}

#[tauri::command]
pub async fn unified_gateway_disable(
    force_backup: Option<bool>,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::disable(force_backup.unwrap_or(false)).await
}

#[tauri::command]
pub async fn unified_gateway_resolve_conflict(
    restore_backup: bool,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::resolve_conflict(restore_backup).await
}

#[tauri::command]
pub async fn unified_gateway_select_grok_accounts(
    account_ids: Vec<String>,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::select_grok_accounts(account_ids).await
}

#[tauri::command]
pub async fn unified_gateway_import_local_grok(
    path: Option<String>,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::import_local_grok(path).await
}

#[tauri::command]
pub async fn unified_gateway_upsert_api_provider(
    draft: crate::models::unified_model_gateway::UnifiedApiProviderDraft,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::upsert_api_provider(draft).await
}

#[tauri::command]
pub async fn unified_gateway_delete_api_provider(
    provider_id: String,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::delete_api_provider(provider_id).await
}

#[tauri::command]
pub async fn unified_gateway_test_provider(
    base_url: String,
    api_key: String,
    wire_api: String,
) -> Result<Vec<String>, String> {
    unified_model_gateway::test_api_provider(base_url, api_key, wire_api).await
}

#[tauri::command]
pub async fn unified_gateway_set_model_enabled(
    model_id: String,
    enabled: bool,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::set_model_enabled(model_id, enabled).await
}

#[tauri::command]
pub async fn unified_gateway_set_routing_policy(
    policy: crate::models::unified_model_gateway::UnifiedRoutingPolicy,
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::set_routing_policy(policy).await
}

#[tauri::command]
pub async fn unified_gateway_migrate_from_router(
) -> Result<crate::models::unified_model_gateway::UnifiedGatewayStateView, String> {
    unified_model_gateway::migrate_from_router().await
}

#[tauri::command]
pub fn codex_router_run_doctor(
) -> Result<crate::modules::codex_router::CodexRouterDoctorReport, String> {
    codex_router::run_doctor()
}

// Codex 命令按职责拆分为以下三个源码片段；这些文件通过 include! 保持原有模块作用域和命令调用路径不变。
// - codex_account_commands.rs：账号、授权、切号、导入导出与配额命令。
// - codex_model_provider_commands.rs：模型供应商配置、连接测试和用量查询命令。
// - codex_local_access_commands.rs：本地 API 服务、API Key 与网关管理命令。
// 各片段中的公开命令仍由本模块统一导出，调用方无需改变。
include!("codex_account_commands.rs");
include!("codex_model_provider_commands.rs");
include!("codex_local_access_commands.rs");
