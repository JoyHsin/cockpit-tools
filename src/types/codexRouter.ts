export interface CodexRouterStatus {
  installed: boolean;
  configured: boolean;
  running: boolean;
  version: string | null;
  enabledProviderCount: number;
  lastError: string | null;
}

export type CodexRouterServiceAction = "start" | "stop" | "restart";
