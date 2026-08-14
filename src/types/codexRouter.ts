export interface CodexRouterStatus {
  installed: boolean;
  configured: boolean;
  running: boolean;
  version: string | null;
  enabledProviderCount: number;
  lastError: string | null;
}

export type CodexRouterServiceAction = "start" | "stop" | "restart";

export interface CodexRouterProvider {
  id: string;
  displayName: string;
  kind: "oauth" | "api" | "openai-compatible" | string;
  configured: boolean;
  cliInstalled: boolean | null;
  cliRunnable: boolean | null;
  action: "ready" | "install" | "login" | "blocked" | "add-key" | string;
  visible: boolean;
  /** Safe display label for the configured credential; never a secret. */
  credentialLabel?: string | null;
  /**
   * True only when Router registry marks this as a non-keyless openai-compatible
   * provider that stores a managed credential. Snapshot kind "api" alone is not enough.
   */
  supportsApiKey?: boolean;
}

export function isCodexRouterApiKeyProvider(
  provider: Pick<CodexRouterProvider, "supportsApiKey" | "kind">,
): boolean {
  if (typeof provider.supportsApiKey === "boolean") {
    return provider.supportsApiKey;
  }
  // Backward-safe fallback for older payloads; prefer supportsApiKey.
  return provider.kind === "api" || provider.kind === "openai-compatible";
}

export interface CodexRouterDoctorCheck {
  status: "ok" | "warn" | "fail" | string;
  name: string;
  detail: string;
  fix: string | null;
}

export interface CodexRouterDoctorReport {
  ok: boolean;
  checks: CodexRouterDoctorCheck[];
}
