export type UnifiedGatewayLifecycle =
  | "disabled"
  | "preparing"
  | "configured"
  | "verifying"
  | "active"
  | "recovery_required";

export type UnifiedProviderType =
  | "official_codex"
  | "grok_oauth"
  | "xai_api"
  | "openai_compatible";

export type UnifiedRoutingMode =
  | "single_account"
  | "weighted"
  | "priority"
  | "backup";

export interface UnifiedModelCapabilities {
  text: boolean;
  streaming: boolean;
  tools: boolean;
  vision: boolean;
  search: boolean;
}

export interface UnifiedModelProvider {
  id: string;
  type: UnifiedProviderType;
  displayName: string;
  enabled: boolean;
  credentialRefIds: string[];
  baseUrl?: string | null;
  wireApi?: string | null;
}

export interface UnifiedCredentialRef {
  id: string;
  type: "grok-oauth" | "api-key";
  accountId?: string | null;
  secretId?: string | null;
  enabled: boolean;
  priority: number;
  weight: number;
  backupOnly: boolean;
  minRemainingPercent?: number | null;
  allowedModels: string[];
  label?: string | null;
}

export interface UnifiedRoutingPolicy {
  mode: UnifiedRoutingMode;
  sessionAffinity: boolean;
  sessionAffinityTtlMs: number;
}

export interface UnifiedGrokAccountOption {
  accountId: string;
  email: string;
  authMode: string;
  eligible: boolean;
  selected: boolean;
  hasGrokCodeAccess: boolean;
  status?: string | null;
  remainingPercent?: number | null;
  usageUpdatedAt?: number | null;
  source: string;
  ineligibleReason?: string | null;
}

export interface UnifiedGrokImportCandidate {
  source: string;
  path: string;
  identity: string;
  email?: string | null;
  expiresAt?: number | null;
  alreadyManaged: boolean;
  importable: boolean;
  reason?: string | null;
}

export interface UnifiedGatewayCatalogEntry {
  id: string;
  displayName: string;
  providerId: string;
  providerType: UnifiedProviderType;
  upstreamModel: string;
  enabled: boolean;
  conflict: boolean;
  capabilities: UnifiedModelCapabilities;
  availability: string;
}

export interface UnifiedGatewayLogEvent {
  at: number;
  level: string;
  code: string;
  message: string;
}

export interface UnifiedProviderHealth {
  providerId: string;
  displayName: string;
  healthy: boolean;
  detail: string;
}

export interface UnifiedGatewayDiagnostics {
  lifecycle: UnifiedGatewayLifecycle;
  running: boolean;
  officialAuthProtected: boolean;
  ownershipMatched: boolean;
  currentConfigHash?: string | null;
  managedConfigHash?: string | null;
  originalConfigHash?: string | null;
  brokerListening: boolean;
  sidecarRunning: boolean;
  lastError?: string | null;
  lastErrorCode?: string | null;
  recentEvents: UnifiedGatewayLogEvent[];
  providerHealth: UnifiedProviderHealth[];
}

export interface UnifiedGatewayConflict {
  present: boolean;
  currentHash?: string | null;
  managedHash?: string | null;
  originalHash?: string | null;
  currentPreview?: string | null;
  managedPreview?: string | null;
}

export interface UnifiedRouterMigrationProvider {
  id: string;
  displayName: string;
  kind: string;
  visible: boolean;
  configured: boolean;
}

export interface UnifiedRouterMigrationPreview {
  routerInstalled: boolean;
  routerConfigured: boolean;
  routerRunning: boolean;
  providers: UnifiedRouterMigrationProvider[];
  matchedGrokAccountIds: string[];
  importableLocalGrok: boolean;
  warning?: string | null;
}

export interface UnifiedGatewayStatus {
  lifecycle: UnifiedGatewayLifecycle;
  running: boolean;
  configured: boolean;
  officialAuthProtected: boolean;
  ownershipMatched: boolean;
  port: number;
  baseUrl?: string | null;
  enabledProviderCount: number;
  enabledModelCount: number;
  grokAccountCount: number;
  lastError?: string | null;
  lastErrorCode?: string | null;
  owner: string;
  conflict: boolean;
  routerDetected: boolean;
  apiServiceActive: boolean;
  supportedCodexRange: string;
  threatModelNote: string;
}

export interface UnifiedGatewayStateView {
  status: UnifiedGatewayStatus;
  providers: UnifiedModelProvider[];
  models: UnifiedGatewayCatalogEntry[];
  grokAccounts: UnifiedGrokAccountOption[];
  importCandidates: UnifiedGrokImportCandidate[];
  credentialRefs: UnifiedCredentialRef[];
  routingPolicy: UnifiedRoutingPolicy;
  diagnostics: UnifiedGatewayDiagnostics;
  conflict: UnifiedGatewayConflict;
  routerMigration: UnifiedRouterMigrationPreview;
}

export interface UnifiedApiProviderDraft {
  displayName: string;
  providerType: UnifiedProviderType;
  baseUrl: string;
  apiKey: string;
  models: string[];
  wireApi?: string | null;
}
