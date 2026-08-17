import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import {
  CircleAlert,
  Download,
  KeyRound,
  Power,
  RefreshCw,
  Route,
  ShieldCheck,
  Wrench,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import * as unifiedGatewayService from "../../services/unifiedModelGatewayService";
import type {
  UnifiedApiProviderDraft,
  UnifiedGatewayStateView,
  UnifiedGrokAccountOption,
} from "../../types/unifiedModelGateway";
import "../codex/CodexRouterStatusCard.css";

export function UnifiedGatewayStatusCard() {
  const { t } = useTranslation();
  const [state, setState] = useState<UnifiedGatewayStateView | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showManager, setShowManager] = useState(false);
  const [selectedGrokIds, setSelectedGrokIds] = useState<string[]>([]);
  const [apiDraft, setApiDraft] = useState<UnifiedApiProviderDraft>({
    displayName: "OpenAI Compatible",
    providerType: "openai_compatible",
    baseUrl: "",
    apiKey: "",
    models: [],
    wireApi: "chat_completions",
  });
  const [apiModelsText, setApiModelsText] = useState("");

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      const next = await unifiedGatewayService.getUnifiedGatewayState();
      setState(next);
      setSelectedGrokIds(
        next.grokAccounts.filter((account) => account.selected).map((account) => account.accountId),
      );
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

  const run = useCallback(
    async (name: string, action: () => Promise<UnifiedGatewayStateView>) => {
      setBusy(name);
      try {
        const next = await action();
        setState(next);
        setSelectedGrokIds(
          next.grokAccounts.filter((account) => account.selected).map((account) => account.accountId),
        );
        setError(null);
      } catch (cause) {
        const message = String(cause).replace(/^Error:\s*/, "");
        setError(message);
        if (message === "CONFIG_CONFLICT") {
          await refresh();
        }
      } finally {
        setBusy(null);
      }
    },
    [refresh],
  );

  const status = state?.status;
  const active = status?.lifecycle === "active";
  const statusClass = active ? "running" : status?.conflict ? "stopped" : "missing";
  const statusLabel = active
    ? t("codex.unifiedGateway.statusActive", "运行中")
    : status?.conflict
      ? t("codex.unifiedGateway.statusConflict", "需要恢复")
      : t("codex.unifiedGateway.statusIdle", "未启用");

  const grokAccounts = state?.grokAccounts ?? [];
  const oauthGrok = useMemo(
    () => grokAccounts.filter((account) => account.authMode === "oauth"),
    [grokAccounts],
  );
  const grokModels = useMemo(
    () => (state?.models ?? []).filter((model) => model.providerType === "grok_oauth"),
    [state],
  );
  const officialModels = useMemo(
    () => (state?.models ?? []).filter((model) => model.providerType === "official_codex"),
    [state],
  );
  const apiModels = useMemo(
    () =>
      (state?.models ?? []).filter(
        (model) =>
          model.providerType === "xai_api" || model.providerType === "openai_compatible",
      ),
    [state],
  );

  const toggleGrok = (account: UnifiedGrokAccountOption) => {
    setSelectedGrokIds((current) =>
      current.includes(account.accountId)
        ? current.filter((id) => id !== account.accountId)
        : [...current, account.accountId],
    );
  };

  return (
    <section className={`codex-router-card ${statusClass}`} aria-busy={loading || Boolean(busy)}>
      <div className="codex-router-card-topline">
        <div className="codex-router-card-icon" aria-hidden="true">
          <Route size={19} />
        </div>
        <div className="codex-router-card-heading">
          <div className="codex-router-card-title-row">
            <h3>{t("codex.unifiedGateway.title", "统一模型网关")}</h3>
            <span className={`codex-router-status ${statusClass}`}>
              <span className="codex-router-status-dot" />
              {statusLabel}
            </span>
          </div>
          <p>
            {t(
              "codex.unifiedGateway.hint",
              "保留官方 Codex 登录，并在同一模型列表中使用已选的 Grok CLI OAuth 账号与 API Provider。",
            )}
          </p>
        </div>
        <button
          type="button"
          className="codex-router-icon-action"
          onClick={() => void refresh()}
          disabled={loading || Boolean(busy)}
          title={t("common.refresh", "刷新")}
        >
          <RefreshCw size={14} className={loading ? "loading-spinner" : ""} />
        </button>
      </div>

      <div className="codex-router-card-body">
        <div className="codex-router-metadata">
          <span>
            {t("codex.unifiedGateway.port", "端口")} <strong>{status?.port ?? "-"}</strong>
          </span>
          <span>
            {t("codex.unifiedGateway.providers", "已启用 Provider")}{" "}
            <strong>{status?.enabledProviderCount ?? 0}</strong>
          </span>
          <span>
            {t("codex.unifiedGateway.models", "模型")}{" "}
            <strong>{status?.enabledModelCount ?? 0}</strong>
          </span>
        </div>
        <div className="codex-router-safety-note">
          <ShieldCheck size={13} />
          <span>
            {status?.officialAuthProtected
              ? t("codex.unifiedGateway.officialProtected", "官方 auth.json 受保护，不会被改写")
              : t("codex.unifiedGateway.officialUnknown", "尚未确认官方登录保护状态")}
          </span>
        </div>
        {status?.routerDetected && (
          <div className="codex-router-empty-note">
            <CircleAlert size={14} />
            <span>
              {t(
                "codex.unifiedGateway.routerDetected",
                "检测到旧 Codex Router。可迁移到统一网关，或仅停用旧路由。旧 Router 不再作为运行时依赖。",
              )}
            </span>
          </div>
        )}
        {status?.threatModelNote && (
          <p className="codex-router-card-heading" style={{ marginTop: 8 }}>
            {status.threatModelNote}
          </p>
        )}
        {(error || status?.lastError) && (
          <div className="codex-router-error" role="alert">
            <CircleAlert size={14} />
            <span>{error || status?.lastError}</span>
          </div>
        )}
      </div>

      <div className="codex-router-card-actions">
        {active ? (
          <button
            type="button"
            className="btn btn-sm btn-secondary"
            onClick={() => void run("disable", () => unifiedGatewayService.disableUnifiedGateway(false))}
            disabled={Boolean(busy)}
          >
            {busy === "disable" ? <RefreshCw size={14} className="loading-spinner" /> : <Power size={14} />}
            {t("codex.unifiedGateway.disable", "停用并恢复官方模式")}
          </button>
        ) : (
          <button
            type="button"
            className="btn btn-sm btn-primary"
            onClick={() => void run("enable", () => unifiedGatewayService.enableUnifiedGateway())}
            disabled={Boolean(busy) || Boolean(status?.apiServiceActive)}
          >
            {busy === "enable" ? <RefreshCw size={14} className="loading-spinner" /> : <Power size={14} />}
            {t("codex.unifiedGateway.enable", "启用统一网关")}
          </button>
        )}
        {status?.routerDetected && (
          <button
            type="button"
            className="btn btn-sm btn-outline"
            onClick={() => void run("migrate", () => unifiedGatewayService.migrateUnifiedGatewayFromRouter())}
            disabled={Boolean(busy)}
          >
            {busy === "migrate" ? <RefreshCw size={14} className="loading-spinner" /> : <Download size={14} />}
            {t("codex.unifiedGateway.migrateRouter", "迁移旧 Router")}
          </button>
        )}
        <button
          type="button"
          className="btn btn-sm btn-outline"
          onClick={() => setShowManager(true)}
          disabled={Boolean(busy)}
        >
          <Wrench size={14} />
          {t("codex.unifiedGateway.manage", "管理")}
        </button>
      </div>

      {showManager &&
        createPortal(
          <div className="modal-overlay codex-router-manager-overlay">
            <div className="modal-content codex-router-manager" style={{ maxWidth: 760 }}>
              <div className="modal-header">
                <h3>{t("codex.unifiedGateway.manageTitle", "统一模型网关")}</h3>
                <button type="button" className="modal-close" onClick={() => setShowManager(false)}>
                  <X size={16} />
                </button>
              </div>
              <div className="modal-body" style={{ display: "grid", gap: 16 }}>
                <section>
                  <h4>{t("codex.unifiedGateway.grokPool", "Grok (OAuth) 账号池")}</h4>
                  <p>
                    {t(
                      "codex.unifiedGateway.grokPoolHint",
                      "选择 Cockpit 已管理的 Grok CLI OAuth 账号。这只会写入账号引用，不会切换默认 ~/.grok 登录，也不会再次弹出 OAuth。",
                    )}
                  </p>
                  {oauthGrok.length === 0 ? (
                    <div className="codex-router-provider-empty">
                      {t("codex.unifiedGateway.noGrokAccounts", "还没有可用于 Codex 的 Grok OAuth 账号。")}
                    </div>
                  ) : (
                    <div style={{ display: "grid", gap: 8 }}>
                      {oauthGrok.map((account) => (
                        <label key={account.accountId} style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
                          <input
                            type="checkbox"
                            checked={selectedGrokIds.includes(account.accountId)}
                            disabled={!account.eligible && !selectedGrokIds.includes(account.accountId)}
                            onChange={() => toggleGrok(account)}
                          />
                          <span>
                            {account.email}
                            {account.remainingPercent != null
                              ? ` · ${t("codex.unifiedGateway.remaining", "剩余")} ${account.remainingPercent}%`
                              : ""}
                            {account.eligible
                              ? ""
                              : ` · ${account.ineligibleReason ?? t("codex.unifiedGateway.ineligible", "当前不可选")}`}
                          </span>
                        </label>
                      ))}
                    </div>
                  )}
                  <div className="codex-router-card-actions" style={{ padding: 0, marginTop: 8 }}>
                    <button
                      type="button"
                      className="btn btn-sm btn-primary"
                      onClick={() =>
                        void run("select-grok", () =>
                          unifiedGatewayService.selectUnifiedGrokAccounts(selectedGrokIds),
                        )
                      }
                    >
                      {t("codex.unifiedGateway.saveGrokPool", "用于 Codex")}
                    </button>
                    <button
                      type="button"
                      className="btn btn-sm btn-outline"
                      onClick={() => void run("import-grok", () => unifiedGatewayService.importUnifiedLocalGrok())}
                    >
                      {t("codex.unifiedGateway.importLocal", "从本机 Grok CLI 导入")}
                    </button>
                  </div>
                  {(state?.importCandidates ?? []).length > 0 && (
                    <div className="codex-router-metadata" style={{ display: "grid", gap: 6 }}>
                      {state?.importCandidates.map((candidate) => (
                        <div key={`${candidate.source}:${candidate.path}`}>
                          <strong>{candidate.source}</strong> · {candidate.identity || candidate.email || "—"}
                          {candidate.importable ? "" : ` · ${candidate.reason ?? ""}`}
                          {candidate.importable && (
                            <button
                              type="button"
                              className="btn btn-sm btn-outline"
                              style={{ marginLeft: 8 }}
                              onClick={() =>
                                void run("import-path", () =>
                                  unifiedGatewayService.importUnifiedLocalGrok(candidate.path),
                                )
                              }
                            >
                              {t("codex.unifiedGateway.importThis", "导入")}
                            </button>
                          )}
                        </div>
                      ))}
                    </div>
                  )}
                </section>

                <section>
                  <h4>{t("codex.unifiedGateway.apiProvider", "API Provider")}</h4>
                  <div style={{ display: "grid", gap: 8 }}>
                    <input
                      value={apiDraft.displayName}
                      onChange={(event) =>
                        setApiDraft((current) => ({ ...current, displayName: event.target.value }))
                      }
                      placeholder={t("codex.unifiedGateway.providerName", "显示名称")}
                    />
                    <input
                      value={apiDraft.baseUrl}
                      onChange={(event) =>
                        setApiDraft((current) => ({ ...current, baseUrl: event.target.value }))
                      }
                      placeholder="https://api.x.ai"
                    />
                    <input
                      type="password"
                      value={apiDraft.apiKey}
                      onChange={(event) =>
                        setApiDraft((current) => ({ ...current, apiKey: event.target.value }))
                      }
                      placeholder={t("codex.unifiedGateway.apiKey", "API Key（不会从旧 Router 读取）")}
                    />
                    <input
                      value={apiModelsText}
                      onChange={(event) => setApiModelsText(event.target.value)}
                      placeholder="grok-4, grok-4.5"
                    />
                    <button
                      type="button"
                      className="btn btn-sm btn-primary"
                      onClick={() =>
                        void run("api-provider", async () => {
                          const next = await unifiedGatewayService.upsertUnifiedApiProvider({
                            ...apiDraft,
                            models: apiModelsText
                              .split(/[,\s]+/)
                              .map((item) => item.trim())
                              .filter(Boolean),
                          });
                          setApiDraft((current) => ({ ...current, apiKey: "" }));
                          return next;
                        })
                      }
                    >
                      <KeyRound size={14} />
                      {t("codex.unifiedGateway.saveProvider", "保存 Provider")}
                    </button>
                  </div>
                </section>

                <section>
                  <h4>{t("codex.unifiedGateway.catalog", "模型目录")}</h4>
                  {(
                    [
                      {
                        title: t("codex.unifiedGateway.officialModels", "官方 Codex"),
                        models: officialModels,
                      },
                      {
                        title: t("codex.unifiedGateway.grokModels", "Grok (OAuth)"),
                        models: grokModels,
                      },
                      {
                        title: t("codex.unifiedGateway.apiModels", "API Provider"),
                        models: apiModels,
                      },
                    ] as const
                  ).map((group) => (
                    <div key={group.title} style={{ marginBottom: 10 }}>
                      <strong>{group.title}</strong>
                      <div style={{ display: "grid", gap: 6, marginTop: 6 }}>
                        {group.models.map((model) => (
                          <label key={model.id} style={{ display: "flex", gap: 8, alignItems: "center" }}>
                            <input
                              type="checkbox"
                              checked={model.enabled}
                              disabled={model.providerType === "official_codex"}
                              onChange={() =>
                                void run(`model:${model.id}`, () =>
                                  unifiedGatewayService.setUnifiedModelEnabled(model.id, !model.enabled),
                                )
                              }
                            />
                            <span>
                              {model.displayName}
                              {model.id !== model.upstreamModel ? ` → ${model.upstreamModel}` : ""}
                              {model.conflict
                                ? ` · ${t("codex.unifiedGateway.conflictId", "已使用 Cockpit 命名空间")}`
                                : ""}
                            </span>
                          </label>
                        ))}
                      </div>
                    </div>
                  ))}
                </section>

                <section>
                  <h4>{t("codex.unifiedGateway.diagnostics", "诊断与恢复")}</h4>
                  <p>
                    {t("codex.unifiedGateway.owner", "当前路由所有者")}：{status?.owner ?? "none"}
                  </p>
                  <p>
                    {t("codex.unifiedGateway.broker", "Credential Broker")}：{" "}
                    {state?.diagnostics.brokerListening
                      ? t("common.enabled", "已启用")
                      : t("common.disabled", "未启用")}
                  </p>
                  {state?.conflict.present && (
                    <div className="codex-router-error">
                      <span>
                        {t(
                          "codex.unifiedGateway.conflictHint",
                          "config.toml 已被手动修改。可以选择只移除 Cockpit 字段，或恢复完整备份。",
                        )}
                      </span>
                      <button
                        type="button"
                        className="btn btn-sm btn-outline"
                        onClick={() =>
                          void run("keep-current", () =>
                            unifiedGatewayService.resolveUnifiedGatewayConflict(false),
                          )
                        }
                      >
                        {t("codex.unifiedGateway.keepCurrent", "保留当前文件并移除 Cockpit 块")}
                      </button>
                      <button
                        type="button"
                        className="btn btn-sm btn-secondary"
                        onClick={() =>
                          void run("restore-backup", () =>
                            unifiedGatewayService.resolveUnifiedGatewayConflict(true),
                          )
                        }
                      >
                        {t("codex.unifiedGateway.restoreBackup", "恢复完整备份")}
                      </button>
                    </div>
                  )}
                  <pre style={{ maxHeight: 180, overflow: "auto", fontSize: 11 }}>
                    {(state?.diagnostics.recentEvents ?? [])
                      .map((event) => `${event.code}: ${event.message}`)
                      .join("\n")}
                  </pre>
                </section>
              </div>
            </div>
          </div>,
          document.body,
        )}
    </section>
  );
}
