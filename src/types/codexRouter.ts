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
  kind: "oauth" | "api" | string;
  configured: boolean;
  cliInstalled: boolean | null;
  cliRunnable: boolean | null;
  action: "ready" | "install" | "login" | "blocked" | "add-key" | string;
  visible: boolean;
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
