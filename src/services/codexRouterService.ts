import { invoke } from "@tauri-apps/api/core";
import type {
  CodexRouterDoctorReport,
  CodexRouterProvider,
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

export async function installCodexRouter(): Promise<CodexRouterStatus> {
  return await invoke("codex_router_install");
}

export async function updateCodexRouter(): Promise<CodexRouterStatus> {
  return await invoke("codex_router_update");
}

export async function enableCodexRouter(): Promise<CodexRouterStatus> {
  return await invoke("codex_router_enable");
}

export async function disableCodexRouter(): Promise<CodexRouterStatus> {
  return await invoke("codex_router_disable");
}

export async function listCodexRouterProviders(): Promise<CodexRouterProvider[]> {
  return await invoke("codex_router_list_providers");
}

export async function setCodexRouterProviderEnabled(
  providerId: string,
  enabled: boolean,
): Promise<CodexRouterProvider[]> {
  return await invoke("codex_router_set_provider_enabled", { providerId, enabled });
}

export async function installCodexRouterProviderCli(
  providerId: string,
): Promise<CodexRouterProvider[]> {
  return await invoke("codex_router_install_provider_cli", { providerId });
}

export async function loginCodexRouterProvider(
  providerId: string,
): Promise<CodexRouterProvider[]> {
  return await invoke("codex_router_login_provider", { providerId });
}

export async function runCodexRouterDoctor(): Promise<CodexRouterDoctorReport> {
  return await invoke("codex_router_run_doctor");
}
