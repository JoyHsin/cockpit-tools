import { invoke } from "@tauri-apps/api/core";
import type {
  CodexRouterServiceAction,
  CodexRouterStatus,
} from "../types/codexRouter";

export async function getCodexRouterStatus(): Promise<CodexRouterStatus> {
  return await invoke("codex_router_get_status");
}

export async function controlCodexRouterService(
  action: CodexRouterServiceAction,
): Promise<CodexRouterStatus> {
  return await invoke("codex_router_control_service", { action });
}
