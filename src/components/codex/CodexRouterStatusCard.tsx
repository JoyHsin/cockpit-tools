import { useCallback, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import {
  CircleAlert,
  Download,
  Eye,
  EyeOff,
  KeyRound,
  Pause,
  Play,
  Power,
  RefreshCw,
  Route,
  ShieldCheck,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import * as codexRouterService from "../../services/codexRouterService";
import type {
  CodexRouterServiceAction,
  CodexRouterDoctorReport,
  CodexRouterProvider,
  CodexRouterStatus,
} from "../../types/codexRouter";
import { isCodexRouterApiKeyProvider } from "../../types/codexRouter";
import "./CodexRouterStatusCard.css";

export function CodexRouterStatusCard() {
  const { t } = useTranslation();
  const [status, setStatus] = useState<CodexRouterStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [action, setAction] = useState<CodexRouterServiceAction | null>(null);
  const [managerAction, setManagerAction] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showManager, setShowManager] = useState(false);
  const [providers, setProviders] = useState<CodexRouterProvider[]>([]);
  const [doctor, setDoctor] = useState<CodexRouterDoctorReport | null>(null);
  const [keyEditorProvider, setKeyEditorProvider] = useState<CodexRouterProvider | null>(null);
  const [keyDraft, setKeyDraft] = useState("");
  const [keyVisible, setKeyVisible] = useState(false);
  const [keySaving, setKeySaving] = useState(false);
  const [keyError, setKeyError] = useState<string | null>(null);

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

  const loadProviders = useCallback(async () => {
    setProviders(await codexRouterService.listCodexRouterProviders());
  }, []);

  const openManager = useCallback(async () => {
    setShowManager(true);
    setError(null);
    if (!status?.installed) return;
    setManagerAction("loading");
    try {
      await loadProviders();
    } catch (cause) {
      setError(String(cause).replace(/^Error:\s*/, ""));
    } finally {
      setManagerAction(null);
    }
  }, [loadProviders, status?.installed]);

  const closeKeyEditor = useCallback(() => {
    setKeyEditorProvider(null);
    setKeyDraft("");
    setKeyVisible(false);
    setKeyError(null);
    setKeySaving(false);
  }, []);

  const openKeyEditor = useCallback((provider: CodexRouterProvider) => {
    setKeyEditorProvider(provider);
    setKeyDraft("");
    setKeyVisible(false);
    setKeyError(null);
    setKeySaving(false);
  }, []);

  const saveProviderKey = useCallback(async () => {
    if (!keyEditorProvider) return;
    const apiKey = keyDraft.trim();
    if (!apiKey) {
      setKeyError(t("codex.router.keyRequiredInput", "请输入 API Key"));
      return;
    }
    setKeySaving(true);
    setKeyError(null);
    try {
      const nextProviders = await codexRouterService.setCodexRouterProviderKey(
        keyEditorProvider.id,
        apiKey,
      );
      setProviders(nextProviders);
      setStatus(await codexRouterService.getCodexRouterStatus());
      setError(null);
      closeKeyEditor();
    } catch (cause) {
      setKeyError(String(cause).replace(/^Error:\s*/, ""));
    } finally {
      setKeySaving(false);
    }
  }, [closeKeyEditor, keyDraft, keyEditorProvider, t]);

  const removeProviderKey = useCallback(
    async (provider: CodexRouterProvider) => {
      const confirmed = window.confirm(
        t(
          "codex.router.removeKeyConfirm",
          "确定移除 {{name}} 的 Cockpit/Router 管理密钥？若环境变量或 Keychain 仍提供凭据，Provider 可能继续显示为已配置。",
          { name: provider.displayName },
        ),
      );
      if (!confirmed) return;
      setManagerAction(`provider-remove-key:${provider.id}`);
      try {
        setProviders(await codexRouterService.removeCodexRouterProviderKey(provider.id));
        setStatus(await codexRouterService.getCodexRouterStatus());
        setError(null);
      } catch (cause) {
        setError(String(cause).replace(/^Error:\s*/, ""));
      } finally {
        setManagerAction(null);
      }
    },
    [t],
  );

  const manage = useCallback(
    async (
      operation:
        | "install"
        | "update"
        | "enable"
        | "disable"
        | "doctor"
        | "provider-install"
        | "provider-login"
        | "provider-toggle",
      provider?: CodexRouterProvider,
    ) => {
      setManagerAction(provider ? `${operation}:${provider.id}` : operation);
      try {
        if (operation === "install") {
          setStatus(await codexRouterService.installCodexRouter());
        } else if (operation === "update") {
          setStatus(await codexRouterService.updateCodexRouter());
        } else if (operation === "enable") {
          setStatus(await codexRouterService.enableCodexRouter());
        } else if (operation === "disable") {
          setStatus(await codexRouterService.disableCodexRouter());
        } else if (operation === "doctor") {
          setDoctor(await codexRouterService.runCodexRouterDoctor());
        } else if (provider && operation === "provider-install") {
          setProviders(await codexRouterService.installCodexRouterProviderCli(provider.id));
        } else if (provider && operation === "provider-login") {
          setProviders(await codexRouterService.loginCodexRouterProvider(provider.id));
        } else if (provider && operation === "provider-toggle") {
          setProviders(
            await codexRouterService.setCodexRouterProviderEnabled(
              provider.id,
              !provider.visible,
            ),
          );
        }
        setError(null);
      } catch (cause) {
        setError(String(cause).replace(/^Error:\s*/, ""));
      } finally {
        setManagerAction(null);
      }
    },
    [],
  );

  const isBusy = loading || action !== null || managerAction !== null || keySaving;
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

  const providerKindLabel = (provider: CodexRouterProvider) => {
    if (provider.kind === "oauth") return "OAuth";
    if (isCodexRouterApiKeyProvider(provider)) {
      return (
        provider.credentialLabel?.trim() ||
        t("codex.router.apiKeyDefaultLabel", "API key")
      );
    }
    // keyless local providers also arrive as kind "api" in Router snapshots
    if (provider.kind === "api" || provider.kind === "openai-compatible") {
      return t("codex.router.keylessManaged", "无需密钥（本地/上游）");
    }
    return t("codex.router.apiKeyManaged", "由上游管理 API Key");
  };

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
                "可由 Cockpit 调用 Codex Router 上游安装器，也会自动识别已有安装。",
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
              {t("codex.router.stop", "停止并恢复")}
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
          <button
            type="button"
            className="btn btn-sm btn-outline"
            onClick={() => void openManager()}
            disabled={isBusy}
          >
            <Wrench size={14} />
            {t("codex.router.manage", "管理")}
          </button>
        </div>
      )}

      {!status?.installed && (
        <div className="codex-router-card-actions">
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={() => void manage("install")}
            disabled={isBusy}
          >
            {managerAction === "install" ? (
              <RefreshCw size={14} className="loading-spinner" />
            ) : (
              <Download size={14} />
            )}
            {t("codex.router.install", "安装 Router")}
          </button>
        </div>
      )}

      {showManager && createPortal(
        <div className="modal-overlay codex-router-manager-overlay">
          <section className="modal-content codex-router-manager" role="dialog" aria-modal="true">
            <header className="codex-router-manager-header">
              <div>
                <div className="codex-router-manager-eyebrow">LOCAL ROUTING</div>
                <h2>Codex Router</h2>
                <p>{t("codex.router.managerHint", "Router 凭据由上游保管；Cockpit 仅调用其公开管理命令。")}</p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={() => {
                  closeKeyEditor();
                  setShowManager(false);
                }}
                aria-label={t("common.close", "关闭")}
              >
                <X size={18} />
              </button>
            </header>

            <div className="codex-router-manager-body">
              <section className="codex-router-manager-section">
                <div className="codex-router-manager-section-head">
                  <div>
                    <span>{t("codex.router.lifecycle", "生命周期")}</span>
                    <strong>{status?.configured ? t("codex.router.configured", "共存路由已验证") : t("codex.router.notConfigured", "尚未接管当前 Codex 配置")}</strong>
                  </div>
                  <div className="codex-router-manager-actions">
                    <button type="button" className="btn btn-sm btn-outline" disabled={isBusy} onClick={() => void manage("doctor")}>
                      <Wrench size={14} />
                      {t("codex.router.doctor", "诊断")}
                    </button>
                    <button type="button" className="btn btn-sm btn-outline" disabled={isBusy} onClick={() => void manage("update")}>
                      <RefreshCw size={14} className={managerAction === "update" ? "loading-spinner" : ""} />
                      {t("codex.router.update", "升级")}
                    </button>
                    {status?.configured ? (
                      <button type="button" className="btn btn-sm btn-secondary" disabled={isBusy} onClick={() => void manage("disable")}>
                        <Power size={14} />
                        {t("codex.router.disable", "停用并恢复")}
                      </button>
                    ) : (
                      <button type="button" className="btn btn-sm btn-primary" disabled={isBusy} onClick={() => void manage("enable")}>
                        <Power size={14} />
                        {t("codex.router.enable", "启用路由")}
                      </button>
                    )}
                  </div>
                </div>
              </section>

              <div className="codex-router-manager-scroll-area">
                <section className="codex-router-manager-section">
                  <div className="codex-router-manager-section-head">
                    <div>
                      <span>{t("codex.router.providers", "Provider")}</span>
                      <strong>{t("codex.router.providersHint", "启用后会刷新 Router 模型目录")}</strong>
                    </div>
                    <button type="button" className="codex-router-icon-action" onClick={() => void loadProviders()} disabled={isBusy} title={t("common.shared.refresh", "刷新")}>
                      <RefreshCw size={14} className={managerAction === "loading" ? "loading-spinner" : ""} />
                    </button>
                  </div>
                  <div className="codex-router-provider-list">
                    {providers.map((provider) => {
                      const providerAction = managerAction?.endsWith(`:${provider.id}`);
                      const apiKeyProvider = isCodexRouterApiKeyProvider(provider);
                      return (
                        <div className="codex-router-provider-row" key={provider.id}>
                          <div>
                            <strong>{provider.displayName}</strong>
                            <span>{providerKindLabel(provider)}</span>
                          </div>
                          <div className="codex-router-provider-actions">
                            {!provider.configured && provider.action === "install" && (
                              <button type="button" className="btn btn-sm btn-outline" disabled={isBusy} onClick={() => void manage("provider-install", provider)}>
                                {providerAction ? <RefreshCw size={13} className="loading-spinner" /> : <Download size={13} />}
                                {t("codex.router.installCli", "安装 CLI")}
                              </button>
                            )}
                            {!provider.configured && provider.action === "login" && (
                              <button type="button" className="btn btn-sm btn-primary" disabled={isBusy} onClick={() => void manage("provider-login", provider)}>
                                {providerAction ? <RefreshCw size={13} className="loading-spinner" /> : <Play size={13} />}
                                {t("codex.router.login", "登录")}
                              </button>
                            )}
                            {apiKeyProvider && (
                              <>
                                <button
                                  type="button"
                                  className="btn btn-sm btn-outline"
                                  disabled={isBusy}
                                  onClick={() => openKeyEditor(provider)}
                                >
                                  <KeyRound size={13} />
                                  {provider.configured
                                    ? t("codex.router.updateKey", "更新密钥")
                                    : t("codex.router.configureKey", "配置密钥")}
                                </button>
                                {provider.configured && (
                                  <button
                                    type="button"
                                    className="btn btn-sm btn-secondary"
                                    disabled={isBusy}
                                    onClick={() => void removeProviderKey(provider)}
                                  >
                                    {managerAction === `provider-remove-key:${provider.id}` ? (
                                      <RefreshCw size={13} className="loading-spinner" />
                                    ) : (
                                      <Trash2 size={13} />
                                    )}
                                    {t("codex.router.removeKey", "移除密钥")}
                                  </button>
                                )}
                              </>
                            )}
                            {provider.configured && (
                              <button type="button" className={`btn btn-sm ${provider.visible ? "btn-secondary" : "btn-outline"}`} disabled={isBusy} onClick={() => void manage("provider-toggle", provider)}>
                                {providerAction && managerAction?.startsWith("provider-toggle:") ? (
                                  <RefreshCw size={13} className="loading-spinner" />
                                ) : (
                                  <Power size={13} />
                                )}
                                {provider.visible ? t("codex.router.hide", "隐藏") : t("codex.router.show", "显示")}
                              </button>
                            )}
                            {!apiKeyProvider && !provider.configured && provider.action === "add-key" && (
                              <span className="codex-router-provider-pending">{t("codex.router.keyRequired", "请在 Router 中配置密钥")}</span>
                            )}
                          </div>
                        </div>
                      );
                    })}
                    {status?.installed && providers.length === 0 && !managerAction && (
                      <div className="codex-router-provider-empty">{t("codex.router.providersEmpty", "暂无 Provider 状态；请刷新或运行诊断。")}</div>
                    )}
                  </div>
                </section>

                {doctor && (
                  <section className="codex-router-manager-section">
                    <div className="codex-router-manager-section-head">
                      <div>
                        <span>{t("codex.router.doctor", "诊断")}</span>
                        <strong>{doctor.ok ? t("codex.router.doctorPassed", "核心检查通过") : t("codex.router.doctorFailed", "发现需要处理的项目")}</strong>
                      </div>
                    </div>
                    <div className="codex-router-doctor-list">
                      {doctor.checks.map((check) => (
                        <div className={`codex-router-doctor-row ${check.status}`} key={`${check.status}-${check.name}`}>
                          <span>{check.status.toUpperCase()}</span>
                          <div><strong>{check.name}</strong><p>{check.detail}</p>{check.fix && <small>{check.fix}</small>}</div>
                        </div>
                      ))}
                    </div>
                  </section>
                )}
              </div>

              {error && <div className="codex-router-error" role="alert"><CircleAlert size={14} /><span>{error}</span></div>}
            </div>
          </section>
        </div>
      , document.body)}

      {keyEditorProvider && createPortal(
        <div className="modal-overlay codex-router-key-overlay" role="presentation">
          <section
            className="modal-content codex-router-key-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="codex-router-key-title"
          >
            <header className="codex-router-key-header">
              <div>
                <div className="codex-router-manager-eyebrow">API KEY</div>
                <h2 id="codex-router-key-title">
                  {keyEditorProvider.configured
                    ? t("codex.router.updateKeyTitle", "更新 Provider 密钥")
                    : t("codex.router.configureKeyTitle", "配置 Provider 密钥")}
                </h2>
                <p>
                  {t(
                    "codex.router.keyEditorHint",
                    "密钥仅通过安全通道写入 Router，不会写入命令行参数、环境变量或日志。",
                  )}
                </p>
              </div>
              <button
                type="button"
                className="modal-close"
                onClick={closeKeyEditor}
                disabled={keySaving}
                aria-label={t("common.close", "关闭")}
              >
                <X size={18} />
              </button>
            </header>

            <div className="codex-router-key-body">
              <span className="codex-router-key-label">
                {t("codex.router.provider", "Provider")}
              </span>
              <div className="codex-router-key-provider">{keyEditorProvider.displayName}</div>

              <label className="codex-router-key-label" htmlFor="codex-router-key-input">
                {t("codex.router.apiKeyField", "API Key")}
              </label>
              <div className="codex-router-key-input-row">
                <input
                  id="codex-router-key-input"
                  className="codex-router-key-input"
                  type={keyVisible ? "text" : "password"}
                  value={keyDraft}
                  autoComplete="new-password"
                  autoCorrect="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  disabled={keySaving}
                  placeholder={t("codex.router.apiKeyPlaceholder", "粘贴供应商 API Key")}
                  onChange={(event) => setKeyDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") {
                      event.preventDefault();
                      void saveProviderKey();
                    }
                  }}
                />
                <button
                  type="button"
                  className="btn btn-secondary icon-only codex-router-key-toggle"
                  disabled={keySaving}
                  onClick={() => setKeyVisible((visible) => !visible)}
                  title={keyVisible ? t("common.hide", "隐藏") : t("common.show", "显示")}
                  aria-label={keyVisible ? t("common.hide", "隐藏") : t("common.show", "显示")}
                >
                  {keyVisible ? <EyeOff size={15} /> : <Eye size={15} />}
                </button>
              </div>

              {keyError && (
                <div className="codex-router-error" role="alert">
                  <CircleAlert size={14} />
                  <span>{keyError}</span>
                </div>
              )}
            </div>

            <footer className="codex-router-key-footer">
              <button
                type="button"
                className="btn btn-secondary"
                disabled={keySaving}
                onClick={closeKeyEditor}
              >
                {t("common.cancel", "取消")}
              </button>
              <button
                type="button"
                className="btn btn-primary"
                disabled={keySaving || !keyDraft.trim()}
                onClick={() => void saveProviderKey()}
              >
                {keySaving ? <RefreshCw size={14} className="loading-spinner" /> : <KeyRound size={14} />}
                {t("common.save", "保存")}
              </button>
            </footer>
          </section>
        </div>
      , document.body)}
    </section>
  );
}
