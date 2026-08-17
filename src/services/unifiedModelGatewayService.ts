import { invoke } from "@tauri-apps/api/core";
import type {
  UnifiedApiProviderDraft,
  UnifiedGatewayStateView,
  UnifiedRoutingPolicy,
} from "../types/unifiedModelGateway";

export async function getUnifiedGatewayState(): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_get_state");
}

export async function enableUnifiedGateway(): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_enable");
}

export async function disableUnifiedGateway(
  forceBackup = false,
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_disable", { forceBackup });
}

export async function resolveUnifiedGatewayConflict(
  restoreBackup: boolean,
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_resolve_conflict", { restoreBackup });
}

export async function selectUnifiedGrokAccounts(
  accountIds: string[],
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_select_grok_accounts", { accountIds });
}

export async function importUnifiedLocalGrok(
  path?: string,
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_import_local_grok", { path: path ?? null });
}

export async function upsertUnifiedApiProvider(
  draft: UnifiedApiProviderDraft,
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_upsert_api_provider", { draft });
}

export async function setUnifiedModelEnabled(
  modelId: string,
  enabled: boolean,
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_set_model_enabled", { modelId, enabled });
}

export async function setUnifiedRoutingPolicy(
  policy: UnifiedRoutingPolicy,
): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_set_routing_policy", { policy });
}

export async function migrateUnifiedGatewayFromRouter(): Promise<UnifiedGatewayStateView> {
  return await invoke("unified_gateway_migrate_from_router");
}
