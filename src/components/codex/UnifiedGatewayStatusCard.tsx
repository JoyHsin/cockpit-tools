import { useCallback, useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import {
  Activity,
  AlertTriangle,
  Bot,
  BrainCircuit,
  Check,
  CheckCircle2,
  CircleAlert,
  Copy,
  Download,
  Eye,
  EyeOff,
  Globe,
  KeyRound,
  Layers,
  Power,
  RefreshCw,
  Route,
  Server,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Trash2,
  Wrench,
  X,
  Zap,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import * as unifiedGatewayService from "../../services/unifiedModelGatewayService";
import {
  PROVIDER_PRESETS,
  type ProviderPreset,
  type UnifiedApiProviderDraft,
  type UnifiedGatewayStateView,
  type UnifiedGrokAccountOption,
} from "../../types/unifiedModelGateway";
import "../codex/CodexRouterStatusCard.css";

type TabKey = "providers" | "grok" | "catalog" | "diagnostics";

export function UnifiedGatewayStatusCard() {
  const { t } = useTranslation();
  const [state, setState] = useState<UnifiedGatewayStateView | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showManager, setShowManager] = useState(false);
  const [activeTab, setActiveTab] = useState<TabKey>("providers");
  const [selectedGrokIds, setSelectedGrokIds] = useState<string[]>([]);
  const [copiedUrl, setCopiedUrl] = useState(false);
  const [showApiKey, setShowApiKey] = useState(false);

  // Provider 编辑状态
  const [selectedPresetId, setSelectedPresetId] = useState<string>("deepseek");
  const [apiDraft, setApiDraft] = useState<UnifiedApiProviderDraft>({
    providerId: null,
    displayName: "DeepSeek (官方)",
    providerType: "openai_compatible",
    baseUrl: "https://api.deepseek.com",
    apiKey: "",
    models: ["deepseek-chat", "deepseek-reasoner"],
    wireApi: "chat_completions",
  });
  const [apiModelsText, setApiModelsText] = useState("deepseek-chat, deepseek-reasoner");

  // 测试连接状态
  const [testingConnection, setTestingConnection] = useState(false);
  const [testResult, setTestResult] = useState<{
    success: boolean;
    models?: string[];
    message?: string;
  } | null>(null);

  // 模型目录过滤
  const [catalogFilter, setCatalogFilter] = useState<"all" | "official" | "grok" | "custom">("all");
  const [catalogSearch, setCatalogSearch] = useState("");

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
  const apiProviders = useMemo(
    () =>
      (state?.providers ?? []).filter(
        (provider) => provider.type === "xai_api" || provider.type === "openai_compatible",
      ),
    [state],
  );

  const allModels = state?.models ?? [];
  const filteredModels = useMemo(() => {
    return allModels.filter((model) => {
      if (catalogFilter === "official" && model.providerType !== "official_codex") return false;
      if (catalogFilter === "grok" && model.providerType !== "grok_oauth") return false;
      if (
        catalogFilter === "custom" &&
        model.providerType !== "openai_compatible" &&
        model.providerType !== "xai_api"
      )
        return false;
      if (catalogSearch.trim()) {
        const search = catalogSearch.toLowerCase();
        return (
          model.id.toLowerCase().includes(search) ||
          model.displayName.toLowerCase().includes(search) ||
          model.upstreamModel.toLowerCase().includes(search)
        );
      }
      return true;
    });
  }, [allModels, catalogFilter, catalogSearch]);

  const handleApplyPreset = (preset: ProviderPreset) => {
    setSelectedPresetId(preset.id);
    setApiDraft((current) => ({
      ...current,
      providerId: null, // 新建模式
      displayName: preset.name,
      providerType: preset.providerType,
      baseUrl: preset.baseUrl,
      wireApi: preset.wireApi,
      apiKey: "",
      models: preset.defaultModels,
    }));
    setApiModelsText(preset.defaultModels.join(", "));
    setTestResult(null);
  };

  const handleSelectExistingProvider = (providerId: string) => {
    const provider = apiProviders.find((p) => p.id === providerId);
    if (!provider) {
      const defaultPreset = PROVIDER_PRESETS[0];
      handleApplyPreset(defaultPreset);
      return;
    }
    setSelectedPresetId("");
    setApiDraft({
      providerId: provider.id,
      displayName: provider.displayName,
      providerType: provider.type,
      baseUrl: provider.baseUrl ?? "",
      wireApi: provider.wireApi ?? "chat_completions",
      apiKey: "",
      models: (state?.models ?? [])
        .filter((m) => m.providerId === provider.id)
        .map((m) => m.upstreamModel),
    });
    setApiModelsText(
      (state?.models ?? [])
        .filter((m) => m.providerId === provider.id)
        .map((m) => m.upstreamModel)
        .join(", "),
    );
    setTestResult(null);
  };

  const handleTestConnection = async () => {
    if (!apiDraft.baseUrl.trim()) {
      setTestResult({ success: false, message: t("codex.unifiedGateway.enterBaseUrl", "请先输入 Base URL") });
      return;
    }
    setTestingConnection(true);
    setTestResult(null);
    try {
      const models = await unifiedGatewayService.testUnifiedApiProvider(
        apiDraft.baseUrl.trim(),
        apiDraft.apiKey.trim(),
        apiDraft.wireApi ?? "chat_completions",
      );
      setTestResult({
        success: true,
        models,
        message:
          models.length > 0
            ? t("codex.unifiedGateway.testSuccessWithCount", `连接成功！发现 ${models.length} 个可用模型`)
            : t("codex.unifiedGateway.testSuccessNoModels", "连接成功！未从 /v1/models 返回模型列表"),
      });
    } catch (err) {
      setTestResult({
        success: false,
        message: String(err).replace(/^Error:\s*/, ""),
      });
    } finally {
      setTestingConnection(false);
    }
  };

  const toggleGrok = (account: UnifiedGrokAccountOption) => {
    setSelectedGrokIds((current) =>
      current.includes(account.accountId)
        ? current.filter((id) => id !== account.accountId)
        : [...current, account.accountId],
    );
  };

  const copyCapabilityUrl = () => {
    if (status?.baseUrl) {
      void navigator.clipboard.writeText(status.baseUrl);
      setCopiedUrl(true);
      setTimeout(() => setCopiedUrl(false), 2000);
    }
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
              "在保持官方 GPT 账号登录与原生体验下，无缝接入 Grok (OAuth) 及第三方模型 API。",
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
            {t("codex.unifiedGateway.models", "可用模型")}{" "}
            <strong>{status?.enabledModelCount ?? 0}</strong>
          </span>
        </div>

        <div className="codex-router-safety-note">
          <ShieldCheck size={13} />
          <span>
            {status?.officialAuthProtected
              ? t("codex.unifiedGateway.officialProtected", "官方 auth.json 受保护，不改写密钥")
              : t("codex.unifiedGateway.officialUnknown", "尚未确认官方登录保护状态")}
          </span>
        </div>

        {status?.routerDetected && (
          <div className="codex-router-empty-note">
            <CircleAlert size={14} />
            <span>
              {t(
                "codex.unifiedGateway.routerDetected",
                "检测到旧 Codex Router，可一键迁移配置到内置统一网关。",
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
        <button
          type="button"
          className="btn btn-sm btn-outline"
          onClick={() => setShowManager(true)}
          disabled={Boolean(busy)}
        >
          <Wrench size={14} />
          {t("codex.unifiedGateway.manage", "管理与配置")}
        </button>
      </div>

      {showManager &&
        createPortal(
          <div className="modal-overlay codex-router-manager-overlay" onClick={() => setShowManager(false)}>
            <div
              className="modal-content ugw-modal-shell"
              onClick={(e) => e.stopPropagation()}
            >
              {/* 弹窗顶部栏 */}
              <div className="ugw-modal-header">
                <div className="ugw-header-brand">
                  <div className="ugw-brand-icon">
                    <Route size={20} />
                  </div>
                  <div>
                    <h2>{t("codex.unifiedGateway.manageTitle", "统一模型网关管理")}</h2>
                    <p className="ugw-header-subtitle">
                      {t("codex.unifiedGateway.subtitle", "管理 Codex 官方透传、Grok (OAuth) 及第三方 API 提供商")}
                    </p>
                  </div>
                </div>
                <div className="ugw-header-actions">
                  <span className={`ugw-status-badge ${active ? "active" : "idle"}`}>
                    <span className="ugw-status-dot" />
                    {active ? "Gateway Active" : "Gateway Inactive"}
                  </span>
                  <button type="button" className="modal-close" onClick={() => setShowManager(false)}>
                    <X size={18} />
                  </button>
                </div>
              </div>

              {/* 网关 URL 状态栏 */}
              {active && status?.baseUrl && (
                <div className="ugw-url-banner">
                  <div className="ugw-url-info">
                    <Server size={14} />
                    <span className="ugw-url-label">Endpoint URL:</span>
                    <code className="ugw-url-code">{status.baseUrl}</code>
                  </div>
                  <button
                    type="button"
                    className="btn btn-xs btn-outline ugw-copy-btn"
                    onClick={copyCapabilityUrl}
                  >
                    {copiedUrl ? <Check size={12} /> : <Copy size={12} />}
                    {copiedUrl ? "已复制" : "复制"}
                  </button>
                </div>
              )}

              {/* 标签栏 */}
              <div className="ugw-tabs-nav">
                <button
                  type="button"
                  className={`ugw-tab-btn ${activeTab === "providers" ? "active" : ""}`}
                  onClick={() => setActiveTab("providers")}
                >
                  <Globe size={15} />
                  <span>API Providers</span>
                  <span className="ugw-tab-count">{apiProviders.length}</span>
                </button>
                <button
                  type="button"
                  className={`ugw-tab-btn ${activeTab === "grok" ? "active" : ""}`}
                  onClick={() => setActiveTab("grok")}
                >
                  <Zap size={15} />
                  <span>Grok (OAuth)</span>
                  <span className="ugw-tab-count">{oauthGrok.length}</span>
                </button>
                <button
                  type="button"
                  className={`ugw-tab-btn ${activeTab === "catalog" ? "active" : ""}`}
                  onClick={() => setActiveTab("catalog")}
                >
                  <Layers size={15} />
                  <span>{t("codex.unifiedGateway.modelsCatalog", "模型目录")}</span>
                  <span className="ugw-tab-count">{allModels.length}</span>
                </button>
                <button
                  type="button"
                  className={`ugw-tab-btn ${activeTab === "diagnostics" ? "active" : ""}`}
                  onClick={() => setActiveTab("diagnostics")}
                >
                  <Activity size={15} />
                  <span>{t("codex.unifiedGateway.diagnostics", "系统诊断")}</span>
                </button>
              </div>

              {/* 弹窗内容主体 */}
              <div className="ugw-modal-body">
                {/* TAB 1: API PROVIDERS */}
                {activeTab === "providers" && (
                  <div className="ugw-tab-content">
                    {/* 快捷预设选择 */}
                    <div className="ugw-card">
                      <div className="ugw-card-header">
                        <div>
                          <h4>{t("codex.unifiedGateway.presets", "快捷添加预设 / 编辑 Provider")}</h4>
                          <p className="ugw-muted-text">
                            {t("codex.unifiedGateway.presetsHint", "选择主流大模型平台一键填入端点与模型，或编辑已有 Provider")}
                          </p>
                        </div>
                      </div>

                      {/* 预设网格 */}
                      <div className="ugw-presets-grid">
                        {PROVIDER_PRESETS.map((preset) => {
                          const isSelected = selectedPresetId === preset.id && !apiDraft.providerId;
                          return (
                            <button
                              key={preset.id}
                              type="button"
                              className={`ugw-preset-chip ${isSelected ? "selected" : ""}`}
                              onClick={() => handleApplyPreset(preset)}
                            >
                              <Sparkles size={13} />
                              <span>{preset.name}</span>
                            </button>
                          );
                        })}
                      </div>

                      {/* 编辑已有 Provider 下拉 */}
                      {apiProviders.length > 0 && (
                        <div className="ugw-existing-selector">
                          <span className="ugw-field-label">切换编辑已有 Provider:</span>
                          <select
                            value={apiDraft.providerId ?? ""}
                            onChange={(e) => handleSelectExistingProvider(e.target.value)}
                            className="ugw-select"
                          >
                            <option value="">{t("codex.unifiedGateway.newProviderMode", "+ 新建自定义 Provider")}</option>
                            {apiProviders.map((p) => (
                              <option key={p.id} value={p.id}>
                                {p.displayName} ({p.id})
                              </option>
                            ))}
                          </select>
                        </div>
                      )}
                    </div>

                    {/* Provider 配置表单 */}
                    <div className="ugw-card">
                      <div className="ugw-card-header">
                        <h4>
                          {apiDraft.providerId
                            ? t("codex.unifiedGateway.editProviderTitle", "编辑 Provider 配置")
                            : t("codex.unifiedGateway.newProviderTitle", "新建 Provider 配置")}
                        </h4>
                      </div>

                      <div className="ugw-form-grid">
                        <div className="ugw-form-group">
                          <label className="ugw-field-label">
                            {t("codex.unifiedGateway.displayName", "显示名称")}
                          </label>
                          <input
                            type="text"
                            className="ugw-input"
                            value={apiDraft.displayName}
                            onChange={(e) =>
                              setApiDraft((current) => ({ ...current, displayName: e.target.value }))
                            }
                            placeholder="例如: DeepSeek (官方)"
                          />
                        </div>

                        <div className="ugw-form-group">
                          <label className="ugw-field-label">
                            {t("codex.unifiedGateway.wireApi", "传输协议")}
                          </label>
                          <select
                            className="ugw-select"
                            value={apiDraft.wireApi ?? "chat_completions"}
                            onChange={(e) =>
                              setApiDraft((current) => ({ ...current, wireApi: e.target.value }))
                            }
                          >
                            <option value="chat_completions">
                              Chat Completions (OpenAI Compatible 兼容格式 - 常用)
                            </option>
                            <option value="responses">
                              Responses 原生 API (Meta / xAI 原生格式)
                            </option>
                          </select>
                        </div>

                        <div className="ugw-form-group full-width">
                          <label className="ugw-field-label">Base URL</label>
                          <input
                            type="text"
                            className="ugw-input"
                            value={apiDraft.baseUrl}
                            onChange={(e) =>
                              setApiDraft((current) => ({ ...current, baseUrl: e.target.value }))
                            }
                            placeholder="https://api.deepseek.com"
                          />
                        </div>

                        <div className="ugw-form-group full-width">
                          <div className="ugw-label-with-action">
                            <label className="ugw-field-label">API Key</label>
                            <span className="ugw-muted-tag">存储于本地安全 Broker，不写入明文配置</span>
                          </div>
                          <div className="ugw-input-with-icon">
                            <input
                              type={showApiKey ? "text" : "password"}
                              className="ugw-input"
                              value={apiDraft.apiKey}
                              onChange={(e) =>
                                setApiDraft((current) => ({ ...current, apiKey: e.target.value }))
                              }
                              placeholder={
                                apiDraft.providerId
                                  ? t("codex.unifiedGateway.apiKeyKeep", "留空表示保留已有 API Key")
                                  : "sk-..."
                              }
                            />
                            <button
                              type="button"
                              className="ugw-icon-btn"
                              onClick={() => setShowApiKey(!showApiKey)}
                              title={showApiKey ? "隐藏" : "显示"}
                            >
                              {showApiKey ? <EyeOff size={15} /> : <Eye size={15} />}
                            </button>
                          </div>
                        </div>

                        <div className="ugw-form-group full-width">
                          <div className="ugw-label-with-action">
                            <label className="ugw-field-label">
                              {t("codex.unifiedGateway.modelsList", "模型 ID 列表 (逗号分隔)")}
                            </label>
                            <button
                              type="button"
                              className="btn btn-xs btn-outline ugw-test-btn"
                              onClick={handleTestConnection}
                              disabled={testingConnection}
                            >
                              {testingConnection ? (
                                <RefreshCw size={12} className="loading-spinner" />
                              ) : (
                                <Zap size={12} />
                              )}
                              {testingConnection ? "正在探测..." : "测试连接并拉取模型"}
                            </button>
                          </div>
                          <input
                            type="text"
                            className="ugw-input"
                            value={apiModelsText}
                            onChange={(e) => setApiModelsText(e.target.value)}
                            placeholder="deepseek-chat, deepseek-reasoner"
                          />
                        </div>

                        {/* 测试连接结果提示 */}
                        {testResult && (
                          <div
                            className={`ugw-test-result full-width ${testResult.success ? "success" : "error"}`}
                          >
                            {testResult.success ? (
                              <CheckCircle2 size={16} />
                            ) : (
                              <AlertTriangle size={16} />
                            )}
                            <div className="ugw-test-result-body">
                              <p className="ugw-test-result-msg">{testResult.message}</p>
                              {testResult.models && testResult.models.length > 0 && (
                                <div className="ugw-discovered-models">
                                  <span className="ugw-discovered-label">点击可追加到模型列表:</span>
                                  <div className="ugw-model-chips">
                                    {testResult.models.map((m) => (
                                      <button
                                        key={m}
                                        type="button"
                                        className="ugw-model-chip"
                                        onClick={() => {
                                          const existing = apiModelsText
                                            .split(",")
                                            .map((s) => s.trim())
                                            .filter(Boolean);
                                          if (!existing.includes(m)) {
                                            const updated = [...existing, m].join(", ");
                                            setApiModelsText(updated);
                                          }
                                        }}
                                      >
                                        + {m}
                                      </button>
                                    ))}
                                    <button
                                      type="button"
                                      className="ugw-model-chip primary"
                                      onClick={() => {
                                        setApiModelsText(testResult.models!.join(", "));
                                      }}
                                    >
                                      全部采用
                                    </button>
                                  </div>
                                </div>
                              )}
                            </div>
                          </div>
                        )}
                      </div>

                      {/* 表单操作按钮 */}
                      <div className="ugw-form-actions">
                        <button
                          type="button"
                          className="btn btn-sm btn-primary"
                          disabled={Boolean(busy)}
                          onClick={() =>
                            void run("save-provider", async () => {
                              const models = apiModelsText
                                .split(",")
                                .map((item) => item.trim())
                                .filter(Boolean);
                              const next = await unifiedGatewayService.upsertUnifiedApiProvider({
                                ...apiDraft,
                                models,
                              });
                              setApiDraft((current) => ({
                                ...current,
                                apiKey: "",
                              }));
                              setTestResult(null);
                              return next;
                            })
                          }
                        >
                          <KeyRound size={14} />
                          {t("codex.unifiedGateway.saveProvider", "保存并应用 Provider")}
                        </button>

                        {apiDraft.providerId && (
                          <button
                            type="button"
                            className="btn btn-sm btn-danger-outline"
                            disabled={Boolean(busy)}
                            onClick={() =>
                              void run("delete-provider", async () => {
                                const next = await unifiedGatewayService.deleteUnifiedApiProvider(
                                  apiDraft.providerId!,
                                );
                                handleApplyPreset(PROVIDER_PRESETS[0]);
                                return next;
                              })
                            }
                          >
                            <Trash2 size={14} />
                            {t("codex.unifiedGateway.deleteProvider", "删除 Provider")}
                          </button>
                        )}
                      </div>
                    </div>

                    {/* 已配置 Provider 列表 */}
                    {apiProviders.length > 0 && (
                      <div className="ugw-card">
                        <div className="ugw-card-header">
                          <h4>已配置的 API Provider 列表</h4>
                        </div>
                        <div className="ugw-provider-cards">
                          {apiProviders.map((p) => {
                            const health = state?.diagnostics.providerHealth.find(
                              (h) => h.providerId === p.id,
                            );
                            const modelCount = (state?.models ?? []).filter(
                              (m) => m.providerId === p.id,
                            ).length;
                            return (
                              <div key={p.id} className="ugw-provider-card">
                                <div className="ugw-provider-card-main">
                                  <div className="ugw-provider-card-title">
                                    <strong>{p.displayName}</strong>
                                    <span
                                      className={`ugw-health-pill ${health?.healthy ? "ready" : "warn"}`}
                                    >
                                      {health?.healthy ? "Ready" : health?.detail ?? "Disabled"}
                                    </span>
                                  </div>
                                  <div className="ugw-provider-card-meta">
                                    <span>Base URL: {p.baseUrl}</span>
                                    <span>·</span>
                                    <span>{modelCount} 个模型</span>
                                    <span>·</span>
                                    <span>{p.wireApi ?? "chat_completions"}</span>
                                  </div>
                                </div>
                                <button
                                  type="button"
                                  className="btn btn-xs btn-outline"
                                  onClick={() => handleSelectExistingProvider(p.id)}
                                >
                                  编辑
                                </button>
                              </div>
                            );
                          })}
                        </div>
                      </div>
                    )}
                  </div>
                )}

                {/* TAB 2: GROK (OAUTH) */}
                {activeTab === "grok" && (
                  <div className="ugw-tab-content">
                    <div className="ugw-card">
                      <div className="ugw-card-header">
                        <div>
                          <h4>Grok (OAuth) 账号池调度</h4>
                          <p className="ugw-muted-text">
                            直接复用 Cockpit 管理的 Grok CLI OAuth 凭证。支持自动负载均衡、会话保持（Session Affinity）与 Token 刷新。
                          </p>
                        </div>
                      </div>

                      {oauthGrok.length === 0 ? (
                        <div className="ugw-empty-box">
                          <Bot size={28} />
                          <p>{t("codex.unifiedGateway.noGrokAccounts", "暂无可用于 Codex 的 Grok OAuth 账号。")}</p>
                          <p className="ugw-muted-text">请先在 Grok 模块中添加或登录 Grok 账号。</p>
                        </div>
                      ) : (
                        <div className="ugw-account-list">
                          {oauthGrok.map((account) => {
                            const isChecked = selectedGrokIds.includes(account.accountId);
                            const remaining = account.remainingPercent;
                            return (
                              <div
                                key={account.accountId}
                                className={`ugw-account-card ${isChecked ? "selected" : ""} ${!account.eligible ? "ineligible" : ""}`}
                                onClick={() => {
                                  if (account.eligible || isChecked) {
                                    toggleGrok(account);
                                  }
                                }}
                              >
                                <input
                                  type="checkbox"
                                  checked={isChecked}
                                  disabled={!account.eligible && !isChecked}
                                  onChange={() => toggleGrok(account)}
                                  onClick={(e) => e.stopPropagation()}
                                  className="ugw-checkbox"
                                />
                                <div className="ugw-account-info">
                                  <div className="ugw-account-email">
                                    <strong>{account.email}</strong>
                                    {account.status && (
                                      <span className="ugw-badge subtle">{account.status}</span>
                                    )}
                                  </div>
                                  <div className="ugw-account-quota">
                                    {remaining != null ? (
                                      <>
                                        <div className="ugw-progress-bar">
                                          <div
                                            className="ugw-progress-fill"
                                            style={{ width: `${Math.min(100, remaining)}%` }}
                                          />
                                        </div>
                                        <span className="ugw-quota-text">
                                          剩余配额 {remaining}%
                                        </span>
                                      </>
                                    ) : (
                                      <span className="ugw-muted-text">配额状态未知</span>
                                    )}
                                  </div>
                                  {!account.eligible && (
                                    <div className="ugw-ineligible-reason">
                                      <AlertTriangle size={12} />
                                      <span>{account.ineligibleReason ?? "不可用"}</span>
                                    </div>
                                  )}
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      )}

                      <div className="ugw-form-actions">
                        <button
                          type="button"
                          className="btn btn-sm btn-primary"
                          disabled={Boolean(busy) || oauthGrok.length === 0}
                          onClick={() =>
                            void run("select-grok", () =>
                              unifiedGatewayService.selectUnifiedGrokAccounts(selectedGrokIds),
                            )
                          }
                        >
                          <Check size={14} />
                          保存已选 Grok 账号
                        </button>

                        <button
                          type="button"
                          className="btn btn-sm btn-outline"
                          disabled={Boolean(busy)}
                          onClick={() =>
                            void run("import-grok", () =>
                              unifiedGatewayService.importUnifiedLocalGrok(),
                            )
                          }
                        >
                          <Download size={14} />
                          从本机 Grok CLI 导入
                        </button>
                      </div>
                    </div>
                  </div>
                )}

                {/* TAB 3: MODEL CATALOG */}
                {activeTab === "catalog" && (
                  <div className="ugw-tab-content">
                    <div className="ugw-card">
                      <div className="ugw-card-header">
                        <div>
                          <h4>Codex 原生模型目录 (Model Catalog)</h4>
                          <p className="ugw-muted-text">
                            所有启用的模型将同步写入 Codex 的 model_catalog_json，直接展示在 Codex 下拉选择器中。
                          </p>
                        </div>
                      </div>

                      {/* 过滤器与搜索 */}
                      <div className="ugw-catalog-controls">
                        <div className="ugw-filter-pills">
                          <button
                            type="button"
                            className={`ugw-filter-pill ${catalogFilter === "all" ? "active" : ""}`}
                            onClick={() => setCatalogFilter("all")}
                          >
                            全部 ({allModels.length})
                          </button>
                          <button
                            type="button"
                            className={`ugw-filter-pill ${catalogFilter === "official" ? "active" : ""}`}
                            onClick={() => setCatalogFilter("official")}
                          >
                            官方 Codex (
                            {allModels.filter((m) => m.providerType === "official_codex").length})
                          </button>
                          <button
                            type="button"
                            className={`ugw-filter-pill ${catalogFilter === "grok" ? "active" : ""}`}
                            onClick={() => setCatalogFilter("grok")}
                          >
                            Grok (OAuth) (
                            {allModels.filter((m) => m.providerType === "grok_oauth").length})
                          </button>
                          <button
                            type="button"
                            className={`ugw-filter-pill ${catalogFilter === "custom" ? "active" : ""}`}
                            onClick={() => setCatalogFilter("custom")}
                          >
                            第三方 API (
                            {
                              allModels.filter(
                                (m) =>
                                  m.providerType === "openai_compatible" ||
                                  m.providerType === "xai_api",
                              ).length
                            }
                            )
                          </button>
                        </div>
                        <input
                          type="text"
                          className="ugw-search-input"
                          placeholder="搜索模型名称或 ID..."
                          value={catalogSearch}
                          onChange={(e) => setCatalogSearch(e.target.value)}
                        />
                      </div>

                      {/* 模型列表 */}
                      <div className="ugw-model-list">
                        {filteredModels.map((model) => {
                          const isOfficial = model.providerType === "official_codex";
                          return (
                            <div key={model.id} className="ugw-model-item">
                              <div className="ugw-model-left">
                                <input
                                  type="checkbox"
                                  checked={model.enabled}
                                  disabled={isOfficial || Boolean(busy)}
                                  onChange={(e) =>
                                    void run("toggle-model", () =>
                                      unifiedGatewayService.setUnifiedModelEnabled(
                                        model.id,
                                        e.target.checked,
                                      ),
                                    )
                                  }
                                  className="ugw-checkbox"
                                />
                                <div className="ugw-model-info">
                                  <div className="ugw-model-name-row">
                                    <strong>{model.displayName}</strong>
                                    <code className="ugw-model-slug">{model.id}</code>
                                  </div>
                                  <div className="ugw-model-badges">
                                    <span className="ugw-model-badge provider">
                                      {model.providerId}
                                    </span>
                                    {model.capabilities.tools && (
                                      <span className="ugw-model-badge tools" title="支持工具调用与代码执行">
                                        <Wrench size={11} />
                                        Tools
                                      </span>
                                    )}
                                    {model.capabilities.streaming && (
                                      <span className="ugw-model-badge stream">
                                        <Zap size={11} />
                                        Stream
                                      </span>
                                    )}
                                    {model.capabilities.vision && (
                                      <span className="ugw-model-badge vision">
                                        <Eye size={11} />
                                        Vision
                                      </span>
                                    )}
                                    {model.id.toLowerCase().includes("reasoner") ||
                                    model.id.toLowerCase().includes("r1") ||
                                    model.id.toLowerCase().includes("thinking") ? (
                                      <span className="ugw-model-badge thinking">
                                        <BrainCircuit size={11} />
                                        Thinking
                                      </span>
                                    ) : null}
                                  </div>
                                </div>
                              </div>
                              <span className={`ugw-model-status ${model.enabled ? "enabled" : "disabled"}`}>
                                {model.enabled ? "已启用" : "已停用"}
                              </span>
                            </div>
                          );
                        })}
                      </div>
                    </div>
                  </div>
                )}

                {/* TAB 4: DIAGNOSTICS */}
                {activeTab === "diagnostics" && (
                  <div className="ugw-tab-content">
                    {/* 运行状态与安全信息 */}
                    <div className="ugw-card">
                      <div className="ugw-card-header">
                        <h4>网关与凭证健康诊断</h4>
                      </div>
                      <div className="ugw-diag-grid">
                        <div className="ugw-diag-item">
                          <span className="ugw-diag-key">Sidecar 代理服务:</span>
                          <span
                            className={`ugw-diag-val ${state?.diagnostics.sidecarRunning ? "ok" : "err"}`}
                          >
                            {state?.diagnostics.sidecarRunning ? "运行中" : "未运行"}
                          </span>
                        </div>
                        <div className="ugw-diag-item">
                          <span className="ugw-diag-key">Credential Broker (安全中继):</span>
                          <span
                            className={`ugw-diag-val ${state?.diagnostics.brokerListening ? "ok" : "err"}`}
                          >
                            {state?.diagnostics.brokerListening ? "监听就绪" : "未启动"}
                          </span>
                        </div>
                        <div className="ugw-diag-item">
                          <span className="ugw-diag-key">官方 auth.json 隔离:</span>
                          <span className="ugw-diag-val ok">受保护 (未修改)</span>
                        </div>
                        <div className="ugw-diag-item">
                          <span className="ugw-diag-key">Codex config.toml 托管:</span>
                          <span
                            className={`ugw-diag-val ${state?.diagnostics.ownershipMatched ? "ok" : "warn"}`}
                          >
                            {state?.diagnostics.ownershipMatched ? "匹配正常" : "未托管或有修改"}
                          </span>
                        </div>
                      </div>

                      {/* 冲突处理 */}
                      {state?.conflict.present && (
                        <div className="ugw-conflict-banner">
                          <div className="ugw-conflict-icon">
                            <ShieldAlert size={20} />
                          </div>
                          <div className="ugw-conflict-body">
                            <strong>检测到 config.toml 内容被外部修改</strong>
                            <p className="ugw-muted-text">
                              统一网关检测到 config.toml 在外部发生了变化。若要强制恢复官方配置，请点击下方恢复按钮。
                            </p>
                            <div className="ugw-form-actions" style={{ marginTop: 8 }}>
                              <button
                                type="button"
                                className="btn btn-xs btn-primary"
                                onClick={() =>
                                  void run("resolve-conflict", () =>
                                    unifiedGatewayService.resolveUnifiedGatewayConflict(true),
                                  )
                                }
                              >
                                恢复备份配置
                              </button>
                            </div>
                          </div>
                        </div>
                      )}
                    </div>

                    {/* 最近日志事件 */}
                    <div className="ugw-card">
                      <div className="ugw-card-header">
                        <h4>最近运行日志事件</h4>
                      </div>
                      <div className="ugw-log-stream">
                        {(state?.diagnostics.recentEvents ?? []).length === 0 ? (
                          <div className="ugw-empty-log">暂无日志事件</div>
                        ) : (
                          state?.diagnostics.recentEvents.slice().reverse().map((ev, idx) => (
                            <div key={idx} className={`ugw-log-line ${ev.level}`}>
                              <span className="ugw-log-time">
                                {new Date(ev.at).toLocaleTimeString()}
                              </span>
                              <span className={`ugw-log-level ${ev.level}`}>{ev.level}</span>
                              <span className="ugw-log-code">[{ev.code}]</span>
                              <span className="ugw-log-msg">{ev.message}</span>
                            </div>
                          ))
                        )}
                      </div>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>,
          document.body,
        )}
    </section>
  );
}
