import { useCallback, useEffect, useState } from "react";
import {
  CircleAlert,
  Pause,
  Play,
  RefreshCw,
  Route,
  ShieldCheck,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import * as codexRouterService from "../../services/codexRouterService";
import type {
  CodexRouterServiceAction,
  CodexRouterStatus,
} from "../../types/codexRouter";
import "./CodexRouterStatusCard.css";

export function CodexRouterStatusCard() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<CodexRouterStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [action, setAction] = useState<CodexRouterServiceAction | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setStatus(await codexRouterService.getCodexRouterStatus());
      setError(null);
    } catch (cause) {
      setError(String(cause).replace(/^Error:\s*/, ""));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const control = useCallback(async (nextAction: CodexRouterServiceAction) => {
    setAction(nextAction);
    try {
      setStatus(await codexRouterService.controlCodexRouterService(nextAction));
      setError(null);
    } catch (cause) {
      setError(String(cause).replace(/^Error:\s*/, ""));
    } finally {
      setAction(null);
    }
  }, []);

  const isBusy = loading || action !== null;
  const statusClass = status?.running
    ? "running"
    : status?.installed
      ? "stopped"
      : "missing";
  const statusLabel = status?.running
    ? t("codex.router.statusRunning", "运行中")
    : status?.installed
      ? t("codex.router.statusStopped", "已停止")
      : t("codex.router.statusMissing", "未安装");

  return (
    <section className={`codex-router-card ${statusClass}`} aria-busy={isBusy}>
      <div className="codex-router-card-topline">
        <div className="codex-router-card-icon" aria-hidden="true">
          <Route size={19} />
        </div>
        <div className="codex-router-card-heading">
          <div className="codex-router-card-title-row">
            <h3>Codex Router</h3>
            <span className={`codex-router-status ${statusClass}`}>
              <span className="codex-router-status-dot" />
              {statusLabel}
            </span>
          </div>
          <p>
            {status?.configured
              ? t(
                  "codex.router.protectedHint",
                  "账号切换会保留已验证的外部模型路由",
                )
              : t(
                  "codex.router.defaultHint",
                  "管理已安装的外部模型本地路由服务",
                )}
          </p>
        </div>
        <button
          type="button"
          className="codex-router-icon-action"
          onClick={() => void refresh()}
          disabled={isBusy}
          title={t("common.shared.refresh", "刷新")}
          aria-label={t("common.shared.refresh", "刷新")}
        >
          <RefreshCw size={14} className={loading ? "loading-spinner" : ""} />
        </button>
      </div>

      <div className="codex-router-card-body">
        {status?.installed ? (
          <>
            <div className="codex-router-metadata">
              <span>
                {t("codex.router.version", "版本")} <strong>{status.version || "-"}</strong>
              </span>
              <span>
                {t("codex.router.providers", "已启用 Provider")} <strong>{status.enabledProviderCount}</strong>
              </span>
            </div>
            <div className="codex-router-safety-note">
              <ShieldCheck size={13} />
              <span>
                {status.configured
                  ? t("codex.router.configured", "共存路由已验证")
                  : t("codex.router.notConfigured", "尚未接管当前 Codex 配置")}
              </span>
            </div>
          </>
        ) : (
          <div className="codex-router-empty-note">
            <CircleAlert size={14} />
            <span>
              {t(
                "codex.router.installHint",
                "请先使用 Codex Router 上游安装器完成安装；Cockpit 会自动识别已有安装。",
              )}
            </span>
          </div>
        )}
        {(error || status?.lastError) && (
          <div className="codex-router-error" role="alert">
            <CircleAlert size={14} />
            <span>{error || status?.lastError}</span>
          </div>
        )}
      </div>

      {status?.installed && (
        <div className="codex-router-card-actions">
          {status.running ? (
            <button
              type="button"
              className="btn btn-sm btn-secondary"
              onClick={() => void control("stop")}
              disabled={isBusy}
            >
              {action === "stop" ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <Pause size={14} />
              )}
              {t("codex.router.stop", "停止")}
            </button>
          ) : (
            <button
              type="button"
              className="btn btn-sm btn-primary"
              onClick={() => void control("start")}
              disabled={isBusy}
            >
              {action === "start" ? (
                <RefreshCw size={14} className="loading-spinner" />
              ) : (
                <Play size={14} />
              )}
              {t("codex.router.start", "启动")}
            </button>
          )}
          <button
            type="button"
            className="btn btn-sm btn-outline"
            onClick={() => void control("restart")}
            disabled={isBusy}
          >
            <RefreshCw size={14} className={action === "restart" ? "loading-spinner" : ""} />
            {t("codex.router.restart", "重启")}
          </button>
        </div>
      )}
    </section>
  );
}
