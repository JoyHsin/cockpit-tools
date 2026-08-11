# Cockpit Tools × Codex Router 融合方案

## 目标与结论

本方案将 [codex-router](https://github.com/duolahypercho/codex-router) 作为 Cockpit Tools 可管理的本地 Router sidecar 接入，而**不**将 Router 的 Node/Python 路由实现移植进 Cockpit 的 Rust 后端。

Cockpit 继续负责 ChatGPT/Codex 账号管理、账号切换和 Codex 配置投影；Router 继续负责外部 Provider、OAuth 和外部模型目录。二者通过一份带所有权标识的配置投影协作，避免两个工具相互覆盖 `~/.codex/config.toml`。

首要使用场景是：用户用 Cockpit Tools 切换 ChatGPT/Codex 账号时，已经启用的 Router 共存模式及其外部模型（例如 Grok OAuth）不能失效。

## 已确认的现状

Cockpit 已具备接入所需的主体能力：

- `src-tauri/src/modules/codex_account.rs` 管理 Codex 账号和 `auth.json` / `config.toml` 投影。内置 OpenAI 账号切换目前会清除 `openai_base_url`、受管模型目录和部分 Provider 配置。
- `src-tauri/src/modules/codex_local_access.rs` 已能生成配置、管理独立 sidecar 生命周期、记录诊断和恢复被接管的 Codex Profile。
- `sidecars/cockpit-cliproxy/` 是现有的 Go 网关；它负责 Cockpit API Service，不应被 Router 改造或取代。
- `docs/CODEX_API_SERVICE_HANDOFF.md` 已定义 API Service 的配置投影、启动、恢复和安全约束，可作为本集成的实现基线。

此前会话已定位到的冲突是：普通 Cockpit OAuth 账号切换会把 Router 为“ChatGPT 登录 + 外部模型”共存模式写入的 `openai_base_url` 删除。删除后账号确实切换成功，但 Codex 会直接走原生 ChatGPT 路径，从而拒绝外部模型。

## 架构

```text
Cockpit UI
  ├─ Router 管理页：安装 / 升级 / 启停 / Provider 开关 / 诊断
  └─ Codex 账号页：继续负责 ChatGPT/Codex 账号切换
          │
          ▼
Tauri / Rust（唯一的 Codex 配置编排者）
  ├─ 写入 auth.json：仅投影当前 Cockpit 选中的 ChatGPT/Codex 账号
  ├─ 验证 Router 受管状态：只读取非敏感的路由身份和版本
  └─ 维护 config.toml：保留或恢复经过验证的 Router 投影
          │                                  │
          │                                  ▼
          │                         codex-router sidecar
          │                           ├─ OpenAI/ChatGPT 路由
          │                           └─ 外部 Provider 与 OAuth（如 Grok）
          ▼
Codex App / CLI 模型选择器
```

### 配置所有权

| 内容 | 唯一写入者 | 说明 |
| --- | --- | --- |
| Cockpit 保存的账号、账号分组、当前选择 | Cockpit | 继续采用已有加密存储和账号切换流程。 |
| Codex `auth.json` | Cockpit | 切号时只更新认证投影，不复制或读取 Router 的供应商密钥。 |
| Router 安装目录、进程、Provider/OAuth 状态 | Router（由 Cockpit 调度） | Router 保持上游支持的 Provider 逻辑，Cockpit 只调用其受支持的管理接口/命令。 |
| `config.toml` 中 Router 受管字段 | Cockpit | Cockpit 在状态验证后保留或更新；不采用文件监听的“事后抢修”。 |
| Cockpit API Service 配置与 `cockpit-cliproxy` | Cockpit | 与 Router 独立，不能在同一 Codex Profile 上同时接管同一个 Base URL。 |

## 第一阶段：账号切换兼容层

### 行为

在将账号投影为 `OpenaiBuiltin` 前，`codex_account.rs` 检查当前 `config.toml` 是否是有效的 Router 共存投影。只有所有条件同时满足时，切号才保留 Router 路由字段：

1. `openai_base_url` 是本地 Router 端点，并含 Router 特有路径标记；
2. `model_provider` 为空或为内置 `openai`，不存在矛盾的自定义 Provider；
3. Router 状态文件存在、可解析且版本受支持；
4. 状态中的 `mode`、`managedProvider`、规范化后的 `managedBaseUrl` 与 `config.toml` 完全一致。

符合条件时：

- 更新 Cockpit 当前账号对应的 `auth.json`；
- 保留 Router 的 `openai_base_url`、模型目录和 Router Provider 配置；
- 不清除 Router 的模型选择能力。

不符合条件时，保留现有行为：清理旧的受管 Provider / 模型目录并回到正常内置 OpenAI 配置。这样普通自定义 URL、过期文件或伪造状态不会被误判为 Router。

### 建议的数据契约

Router 需要提供一个非敏感状态文件（名称以 Router 实际公开契约为准），内容至少包括：

```json
{
  "version": 3,
  "mode": "root-openai",
  "managedProvider": "openai",
  "managedBaseUrl": "http://127.0.0.1:<port>/_codex-router/<capability>/v1",
  "ownershipId": "<opaque-id>"
}
```

该文件不得包含 access token、refresh token、API Key 或完整 OAuth 载荷。Cockpit 只将它作为“可安全读取的配置所有权证明”，并将端点规范化后再比较。

### 回归测试

必须新增以下 Rust 单元测试：

- 有效 Router 共存状态下切换 OAuth 账号：`openai_base_url`、模型目录和 Router Provider 仍存在；账号认证已更新。
- 仅含 Router 风格 URL、但无有效状态文件：切换后应清除 `openai_base_url`。
- 状态版本、模式、Provider 或 Base URL 任一不匹配：切换后应清除 Router 投影。
- 自定义 API Provider、Cockpit API Service、Router 未启用：现有逻辑不变。

## 第二阶段：Cockpit 托管 Router

将 Router 接入一个独立的“Codex Router”功能页。该页面不应把 Provider 实现搬进前端，而应围绕 Router 公开的安装与诊断接口建立受控管理能力。

### 最小可用功能

1. 检测环境与已安装版本：Node/Python/`uv` 等依赖由 Router 的官方 doctor 规则判定。
2. 安装、升级、启用、禁用、启动、停止和健康检查。
3. 显示 Router 的实际监听地址、运行状态、最后错误和版本；日志只显示已脱敏诊断。
4. 读取已启用 Provider 和可用模型摘要，不读取或展示 Provider 凭据。
5. 启动由 Router 提供的 OAuth 登录流程；浏览器授权在用户侧完成，Cockpit 不收集验证码或令牌。
6. 启用前备份当前 Codex Profile；禁用时恢复 Cockpit 最后保存的非 Router 配置。

### 生命周期要求

- 使用受控子进程或平台服务管理 Router；禁止依赖脆弱的轮询文件监听来重写 Codex 配置。
- 每次启用、切换 Cockpit 账号、升级、禁用后都应执行一次状态一致性校验。
- 需要明确 Router 与 Cockpit API Service 的互斥关系：同一 Profile 同一时刻只能有一个组件接管主路由；UI 必须显示当前接管者及切换后果。
- Router 启动时应继承 Cockpit 已解析且脱敏处理过的网络代理策略，包含本地地址 `NO_PROXY` 规则；不能假定桌面应用继承终端环境变量。

## 第三阶段：体验完善

- 在模型列表中标识“原生 OpenAI / Router 外部模型 / Cockpit API Service 模型”。
- 对外部模型 OAuth 过期、上游 429、Router 未运行、配置所有权不匹配分别给出可执行提示，避免将所有错误归为“切号失败”。
- 在账号切换完成后进行轻量诊断：验证认证投影与 Router 路由仍可用，不主动发送计费模型请求。
- 可选地提供“修复 Router 共存配置”按钮；它必须先展示将要恢复的配置来源和影响范围。

## 不做的事情

- 不把 Codex Router 的 Node/Python 代码整体翻译到 Rust。
- 不把 OAuth token、Provider API Key 写入 Cockpit 的账号记录、日志、诊断上传或 Git 仓库。
- 不允许 Cockpit API Service 和 Router 同时修改同一 Profile 的主 `openai_base_url`。
- 不把普通自定义 Base URL 误认成 Router 并盲目保留。
- 不通过后台文件监听在用户手动关闭 Router 后自动把它重新打开。

## 实施顺序与验收

| 阶段 | 产出 | 验收标准 |
| --- | --- | --- |
| 1 | 账号切换兼容层与单测 | 启用 Router 后连续切换两个 ChatGPT/Codex 账号，外部模型仍可见；伪造状态不会被保留。 |
| 2 | Router 生命周期、安装与诊断页 | 新装、升级、启停、禁用/恢复可重复执行；所有操作不泄露凭据。 |
| 3 | Provider/OAuth 管理与状态提示 | OAuth 失效、Router 未运行、上游限流、配置冲突都能准确归因。 |
| 4 | 跨平台与回归 | macOS、Windows、Linux 上覆盖 Profile 路径、服务管理、代理与恢复；通过 Rust、前端及 sidecar 回归测试。 |

每次涉及配置投影、模型目录、Base URL、Provider 或认证的改动，都需要同时覆盖：

1. Codex Profile 接管与恢复；
2. 原生 OpenAI 请求路径；
3. Router 外部模型请求路径；
4. 账号切换后的配置一致性；
5. Router 未运行或状态无效时的安全降级。

## 风险与待确认项

- Router 的状态文件、安装命令、服务协议和模型目录格式必须以其上游稳定 API/CLI 为准；不要依赖未公开的内部文件布局。
- Codex 对 ChatGPT 登录和自定义 Provider 的限制可能随版本变化，升级 Codex 时须重跑共存模式回归。
- Router、Provider 和 OAuth 服务各自受其服务条款及限流规则约束；路由接通不代表第三方 OAuth 会话或额度有效。
- 首个代码改动应只实现第一阶段，并在具备 Rust 工具链的 CI/开发机上运行 `cargo fmt` 与定向单测后再继续 UI 和生命周期功能。

## 相关文件

- `src-tauri/src/modules/codex_account.rs`：账号投影与 Router 共存状态校验的落点。
- `src-tauri/src/modules/codex_local_access.rs`：本地网关生命周期、Profile 接管/恢复和诊断能力的可复用基础。
- `src-tauri/src/commands/codex.rs`：新增 Tauri 命令的边界。
- `src/services/codexLocalAccessService.ts` 与 `src/types/codexLocalAccess.ts`：前端桥接与契约。
- `docs/CODEX_API_SERVICE_HANDOFF.md`：既有 API Service 的实现约束与测试清单。
