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
  providerId?: string | null;
  displayName: string;
  providerType: UnifiedProviderType;
  baseUrl: string;
  apiKey: string;
  models: string[];
  wireApi?: string | null;
}

export interface ProviderPreset {
  id: string;
  name: string;
  providerType: UnifiedProviderType;
  baseUrl: string;
  wireApi: "chat_completions" | "responses";
  defaultModels: string[];
  description: string;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "deepseek",
    name: "DeepSeek (官方)",
    providerType: "openai_compatible",
    baseUrl: "https://api.deepseek.com",
    wireApi: "chat_completions",
    defaultModels: ["deepseek-chat", "deepseek-reasoner"],
    description: "DeepSeek V3 极速编程与 R1 深度思考模型",
  },
  {
    id: "moonshot-cn",
    name: "Moonshot Kimi (中国站)",
    providerType: "openai_compatible",
    baseUrl: "https://api.moonshot.cn/v1",
    wireApi: "chat_completions",
    defaultModels: ["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k", "kimi-k3"],
    description: "Moonshot AI Kimi 长上下文模型",
  },
  {
    id: "moonshot-intl",
    name: "Moonshot Kimi (国际站)",
    providerType: "openai_compatible",
    baseUrl: "https://api.moonshot.ai/v1",
    wireApi: "chat_completions",
    defaultModels: ["kimi-k3", "moonshot-v1-128k"],
    description: "Moonshot Platform 国际版",
  },
  {
    id: "siliconflow",
    name: "SiliconFlow (硅基流动)",
    providerType: "openai_compatible",
    baseUrl: "https://api.siliconflow.cn/v1",
    wireApi: "chat_completions",
    defaultModels: ["deepseek-ai/DeepSeek-V3", "deepseek-ai/DeepSeek-R1", "Qwen/Qwen2.5-72B-Instruct"],
    description: "高并发模型托管与推理加速 API",
  },
  {
    id: "dashscope",
    name: "Aliyun 百炼 (通义千问兼容模式)",
    providerType: "openai_compatible",
    baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    wireApi: "chat_completions",
    defaultModels: ["qwen-max", "qwen-plus", "qwen-turbo", "deepseek-v3", "deepseek-r1"],
    description: "阿里云百炼大模型兼容模式",
  },
  {
    id: "zhipu",
    name: "Zhipu AI (智谱清言)",
    providerType: "openai_compatible",
    baseUrl: "https://open.bigmodel.cn/api/paas/v4",
    wireApi: "chat_completions",
    defaultModels: ["glm-4-plus", "glm-4-flash", "glm-4-long", "glm-4-0520"],
    description: "智谱 GLM 系列大模型",
  },
  {
    id: "xai",
    name: "xAI API (Grok 官方 API)",
    providerType: "xai_api",
    baseUrl: "https://api.x.ai",
    wireApi: "chat_completions",
    defaultModels: ["grok-2-latest", "grok-2-vision-latest", "grok-beta"],
    description: "xAI 官方 API Key 接入",
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    providerType: "openai_compatible",
    baseUrl: "https://openrouter.ai/api/v1",
    wireApi: "chat_completions",
    defaultModels: ["anthropic/claude-3.5-sonnet", "deepseek/deepseek-r1", "openai/gpt-4o"],
    description: "全球大模型聚合路由网关",
  },
  {
    id: "ollama",
    name: "Ollama (本地私有化)",
    providerType: "openai_compatible",
    baseUrl: "http://127.0.0.1:11434/v1",
    wireApi: "chat_completions",
    defaultModels: ["deepseek-r1:latest", "qwen2.5-coder:latest", "llama3.3:latest"],
    description: "本地运行的开源大模型服务",
  },
];

