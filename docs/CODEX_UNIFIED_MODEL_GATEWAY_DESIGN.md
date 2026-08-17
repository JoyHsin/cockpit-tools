# Cockpit 统一模型网关完整方案

**状态：提案**
**目标版本：首个支持完整模型路由的 Cockpit 发布版**

## 1. 决策摘要

Cockpit 将提供一个内置的 **统一模型网关（Unified Model Gateway）**，取代用户侧安装、运行和升级 Codex Router 的路径。

该网关是现有 `cockpit-cliproxy` 的受控扩展，由 Cockpit 随安装包交付。它让 Codex 在不退出、不覆盖官方 ChatGPT/Codex 登录的前提下，在同一模型选择器中使用：

- 官方 Codex 模型；
- Grok CLI OAuth 模型（首期必须完整支持）；
- API Key 型 OpenAI-compatible Provider；
- xAI API Provider。

不再要求最终用户安装 Git、Node、Python、uv 或 Codex Router。现有 Codex Router 仅作为迁移来源，不能成为新功能的运行时依赖。

## 2. 用户结果

用户在 Codex 中依旧保持官方账号登录。模型下拉列表同时显示官方模型和已启用的第三方模型：

```text
GPT / Codex 官方账号
├─ GPT-5.x / Codex 官方模型      → 使用官方账号与官方额度
├─ Grok 4.5 (OAuth)              → 使用 Cockpit 已选 Grok CLI OAuth 账号
├─ Grok 4.6 (OAuth)              → 使用 Cockpit 已选 Grok CLI OAuth 账号
└─ 已启用的 API Provider 模型    → 使用该 Provider 的 API Key
```

选择 Grok 时不再弹出第二次 OAuth：用户在左侧 **Grok CLI** 已登录的账号会出现在 Codex 的 `Grok (OAuth)` Provider 下拉框中。选择该账号只创建凭据引用，不切换或覆写用户默认 `~/.grok` 登录。

若用户只在系统 Grok CLI 登录、尚未被 Cockpit 管理，界面提供“从本机 Grok CLI 导入”；这是安全导入已有会话，不是重新登录。

## 3. 范围与边界

### 必须交付

1. 保留 ChatGPT/Codex 官方登录，官方请求继续使用该登录。
2. Grok OAuth：复用 Cockpit 现有账号、导入、刷新、额度和多账号能力。
3. 在 Codex 模型目录中合并展示官方模型与 Grok 模型。
4. Grok 多账号池：单账号、自动、优先级、额度耗尽/授权失效后的切换。
5. OpenAI-compatible API Provider 与 xAI API Provider：API Key、模型发现/手工模型、流式 Responses 路由。
6. 统一生命周期、诊断、日志、更新、恢复与完整卸载。
7. macOS Apple Silicon、macOS Intel、Windows、Linux 的安装包内置运行时。

### 明确不做

- 不读取、重写或登出官方 Codex 的 `auth.json` 以接入第三方模型。
- 不要求用户为了本功能安装 Codex Router、Node、Python、Git、uv 或 Grok CLI。
- 不将 refresh token、API Key 写入 `config.toml`、模型目录、日志、诊断或 Git。
- 不让“选择用于 Codex 的 Grok 账号”改变外部 Grok CLI 的默认账号。
- 不允许旧 Router 与统一网关同时接管同一 Codex Profile。

## 4. 总体架构

```text
Codex App / CLI
  │  官方 auth + model catalog
  ▼
Cockpit 统一模型网关（bundled cockpit-cliproxy）
  ├─ 官方路由：保留客户端的官方认证头，转发至 ChatGPT/Codex
  ├─ Grok OAuth Adapter：按 accountId 向 Credential Broker 取短时 access token
  ├─ OpenAI-compatible Adapter：按 Provider 引用使用 API Key
  ├─ 模型目录与能力注册表
  └─ 用量、健康、限流、会话亲和和故障切换
             ▲
             │ 仅回传短时 token；refresh token 永不出 Rust 进程
Cockpit Rust Credential Broker
  ├─ 官方 Codex 账号投影与配置备份
  ├─ Grok OAuth 账号库、导入与刷新
  ├─ API Key 安全存储
  └─ 网关配置、审计和恢复
```

Broker 不监听 TCP loopback，而使用用户私有 IPC：macOS/Linux 使用 `$APP_DATA/runtime/credential-broker.sock` Unix socket，Windows 使用带当前用户 SID ACL 的 named pipe。socket 目录/文件权限为当前用户独占；每次网关启动都会重新创建。

启动时 Rust 为一个 sidecar 进程生成随机会话密钥与一次性 nonce，通过继承的匿名管道（macOS/Linux 的受限文件描述符、Windows 的不可继承端命名管道 handle）传递；不通过环境变量或命令行传递。连接必须完成 `Hello(protocolVersion, childPid, nonce, HMAC)` 握手；Broker 使用 Unix peer credentials 或 Windows named-pipe client PID/SID 验证调用者，nonce 只接受一次。之后每次调用都带递增序号和 HMAC，超时、sidecar 重启或 PID 不匹配立即撤销会话。IPC 不提供“读取任意 secret”；仅允许已启用路由在请求期获取其绑定账号的一枚短时 access token。

Gateway 的客户端入口也必须认证。受管 Codex Profile 写入不可预测的 profile capability 路径，并仅接受该路径上的请求；官方模型还必须带有由 Codex 发出的官方认证。LAN 模式不复用 capability 路径，必须改用独立 API Key 且默认不允许调用有 OAuth/API 凭据的外部模型。威胁模型明确为：防止网络、其他 OS 用户和意外本机程序访问；与 Cockpit 同一 OS 用户且可读取其私有配置的恶意进程不属于此桌面应用可单独防御的边界，必须在文档和 UI 中说明。

## 5. Codex 配置模型

### 5.1 不替换官方账号

激活统一网关时，Cockpit：

1. 原子备份当前 Codex Profile 的 `config.toml` 与其受管状态；
2. 保持官方 `auth.json`；
3. 保持根 Provider 为 `openai`；
4. 写入一个由 Cockpit 所有权标识保护的 loopback `openai_base_url`；
5. 配置 `requires_openai_auth = true`，以便官方认证仍由 Codex 发起；
6. 写入 Cockpit 生成的合并模型目录；
7. 写入 `unified-gateway-state.json`，记录版本、端口、配置摘要、所有权 ID 与原始配置备份引用。

官方模型请求由网关透传到官方路径，不能被第三方 Provider 重写。第三方模型由模型注册表匹配后才进入相应 Adapter。

### 5.2 所有权与恢复

只有 `config.toml`、本地端点和 `unified-gateway-state.json` 三者完全匹配时，Cockpit 才认为该配置由自己管理。任何不匹配均停止接管并提示恢复，不猜测、不覆盖用户自己的 Provider 配置。

配置迁移是显式状态机：`disabled → preparing → configured → verifying → active`；任一步失败进入 `recovery_required`。每次写入记录原文件 hash、受管块 hash 和更新时间。停用、升级失败或健康检查失败时，Cockpit 只移除自己拥有的 TOML 字段与模型目录引用，而不是盲目覆盖整份备份。若当前文件 hash 与受管 hash 都不匹配，进入冲突界面：用户可查看差异、保留当前文件后仅移除 Cockpit 块，或恢复完整备份。官方账号登录不受影响。

### 5.3 Codex 兼容性契约（发布门禁）

本功能不能假设任意 Codex 版本都能透传官方认证。首发只支持经自动化实机验证的 Codex App/CLI 版本范围；该范围与 Cockpit 版本一起发布并在启动时检测。每次 Codex 大版本升级均要重新验证以下契约：

| 客户端行为 | 网关处理 | 不满足时的动作 |
| --- | --- | --- |
| `GET /v1/models` | 返回合并后的官方与外部模型目录 | 不激活统一网关 |
| `POST /v1/responses` 与 compact | 官方 ID 保留官方认证并透传；外部 ID 送至对应 Adapter | 未知模型返回明确 404/可操作提示 |
| Responses 流式 SSE | 保留事件顺序、request ID、取消语义 | 首字节后只发送终止 error event，不重写 HTTP 状态 |
| Responses WebSocket（若客户端启用） | 官方模型透传；外部模型仅在 Adapter 已验证时开放 | 否则声明不支持，不静默转发 |
| 模型目录配置 | 只使用经版本测试的配置键、JSON schema 和原子文件替换 | schema 不匹配时保持官方原配置 |

发布前必须有一个真实 Codex 冒烟测试：用户已登录官方账号时，官方模型与已选择的 Grok 模型均在同一选择器中出现并成功完成最小 Responses 请求。没有通过该测试的 Codex 版本不允许开启该模式。

## 6. Grok OAuth 设计

### 6.1 账号来源与选择

`Grok (OAuth)` Provider 的账号来源按以下优先级展示：

1. Cockpit 已管理的 Grok OAuth 账号；
2. 已检测到的系统 Grok CLI profile，可一键导入；
3. “登录新 Grok 账号”，复用现有 Device OAuth Flow。

Codex Provider 页使用多选账号池，默认按“健康优先、剩余额度优先、会话亲和”路由。管理员可为每个账号设置启用状态、优先级、仅备用、模型限制与最小剩余额度。

仅 `auth_mode = oauth` 且可用于 Grok Code 的账号可加入 `Grok (OAuth)` 池。API Key 账号显示为独立 `xAI API` Provider，不能伪装为 OAuth。

本机导入按以下顺序发现并要求用户确认：Cockpit 已管理的独立 profile、用户配置的 `grok_cli_path` 所属 profile、macOS/Linux 的 `$HOME/.grok/auth.json`、Windows 的 `%USERPROFILE%\\.grok\\auth.json`。每个候选显示来源、账号标识、到期时间和“将用于 Codex”的影响，不显示 token。未知格式、无法确认所属用户、权限不安全或不含 refresh token 的文件不能导入。

导入创建一个 **共享会话绑定**：Cockpit 将会话安全复制到自己的账号库，并记录源文件、账号强身份和导入时凭据指纹。刷新 token 后，Cockpit 先重新读取源文件；只有源文件仍属于同一账号且匹配上一次已知会话时，才原子地镜像新的 access/refresh token，同时保留无关 registry 条目。它不会切换该 CLI 的默认账号，也不会覆盖另一个账号的会话。若来源已被另一进程轮换，Cockpit 优先采纳新凭据；无法安全对齐时暂停该绑定并提示用户重新导入/授权。这样避免 refresh-token rotation 让原 Grok CLI 失效。

### 6.2 凭据链路

现有 `grok_oauth` 和 `grok_account` 模块是唯一的 refresh token 所有者：

- 选择账号时保存 `account_id` 引用，不复制 token；
- 每个请求前，Broker 调用“获取可用 access token”内部 API；到期前刷新，并持久化旋转后的 refresh token；
- Broker 将短时 access token 仅返回给 sidecar 的内存请求；
- sidecar 不持久化 Grok refresh token；退出后不留 token 文件；
- API Key 不通过 IPC 返回给 sidecar。sidecar 对 API-key Provider 发起 `ExecuteProviderRequest(providerId, modelRoute, request)`；Broker 校验 route 与启用的 Provider 后在 Rust 侧注入 API Key、创建上游连接，并以受控流将响应返回 sidecar。sidecar 永远拿不到长期 API Key；
- 401 / `invalid_grant` 标记该账号需要重新授权，自动换下一个合格账号；若无账号，明确返回“Grok 需要重新授权”，不影响官方模型。

现有账号资料已经支持独立 `GROK_HOME`，其语义必须保留：Codex 使用选择的账号不会调用 `inject_to_default`，从而不会改写用户 CLI 默认会话。

### 6.2.1 账号池与重试规则

账号选择顺序固定为：模型允许 → 已授权/健康 → 未达最小额度阈值 → 会话亲和 → 优先级/权重。一个新会话在第一个成功响应前选定账号，并记录脱敏 session affinity；同一会话的 tool result 必须使用同一账号。

| 时点/结果 | 行为 |
| --- | --- |
| 上游尚未收到请求，连接失败 | 可选择下一个账号重试一次 |
| 401 且 token 可刷新 | 刷新同一账号后重试一次 |
| 401 / `invalid_grant` | 标记账号“需重新授权”，在未输出响应前选择备用账号一次 |
| 429 或确定额度耗尽 | 记录 cooldown/额度状态，在未输出响应前选择备用账号一次 |
| 已接收上游响应首字节或已发送 SSE 事件 | 不切换账号；向当前流发送规范化终止错误 |
| 超时且已写出请求 body | 不自动重试，避免非幂等工具执行重复 |

refresh 操作按账号 ID 加锁；刷新后的 token 先持久化再释放锁。额度数据带采样时间和 TTL，过期时不能当作“仍有额度”的依据。

### 6.3 协议 Adapter

在 Go sidecar 新增 `grokOAuthExecutor`。它接收 Codex Responses 请求，处理并测试：

- 模型 ID、推理等级与 Grok 支持范围；
- 流式 SSE、取消和重试；
- tool/function 定义、重复函数名、tool choice；
- 会话 ID 与请求 ID；
- 上游认证刷新后的单次安全重试；
- 上游 401、429、5xx、超时的分类、退避与账号池切换；
- 文本、图片/视觉和服务端搜索能力的 capability gating。

模型、能力和最大推理等级不能只写死在 UI。注册表须带 `provider`, `route`, `upstreamModel`, `capabilities`, `reasoningEfforts`, `contextWindow`, `availability` 字段；运行时可更新的模型必须先通过兼容测试后才发布给 Codex。

## 7. Provider 与模型注册表

新增持久化实体 `UnifiedModelProvider`、`UnifiedModel`、`CredentialRef`、`RoutingPolicy`：

```text
Provider: id, type, displayName, enabled, credentialRefs, policy
Model:    id, displayName, providerId, upstreamModel, capabilities, efforts
Ref:      type(grok-oauth | api-key), accountId/secretId, enabled, priority
```

模型 ID 必须稳定且全局去重。官方 ID 绝不改写；外部模型出现冲突时使用 Cockpit 命名空间并在 UI 显示真实上游名。模型目录每次变更都同时更新“可见模型”和“请求路由表”，禁止出现下拉可选但无法路由的模型。

首个完整发布至少内置：

| Provider | 认证 | 模型来源 | 运行状态 |
| --- | --- | --- | --- |
| 官方 Codex | Codex 官方登录 | 原生目录 | 必须保留 |
| Grok OAuth | Cockpit Grok 账号池 | Cockpit 验证注册表 | 必须交付 |
| xAI API | API Key | xAI / 手工注册 | 必须交付 |
| OpenAI-compatible | API Key | `/models` + 手工补充 | 必须交付 |

Anthropic 与 Gemini 是统一注册表的后续 Adapter，不影响本方案“完整交付”的定义；在各自的认证、请求映射与实机验收完成前，不在模型目录中宣称可用。

对每个非 Responses 协议的 Provider 必须有独立请求/响应事件映射：输入、reasoning、tool call ID、tool output、取消、SSE completion/error 和 usage 都要逐项定义并测试。能力不兼容时在 Codex 侧尽早返回可操作错误，而不是静默降级。

## 8. UI 与使用流程

### 8.1 新的入口

将 Codex 页面现有的 `Codex Router` 卡迁移为 **统一模型网关**：状态、端口、官方账号保护状态、已启用 Provider 数、诊断和恢复入口都在此处。旧 Router 仅显示“检测到旧安装，可迁移/停用”。

Provider 管理页提供：

1. **Grok (OAuth)**：账号下拉、多选池、导入本机 CLI、刷新、重新授权、模型开关和额度状态；
2. **API Provider**：密钥、Base URL、发现模型、手工模型、能力声明和连通性测试；
3. **模型目录**：官方/第三方来源、模型能力、当前可选性和冲突提示；
4. **路由策略**：单账号、加权、优先级、备用、最小额度和会话亲和；
5. **诊断与恢复**：配置所有权、进程、各 Provider 健康、最近错误和“一键安全恢复官方模式”。

### 8.2 Grok 账号无重复登录流程

```text
Grok CLI 页已有账号
        │
        ├─ 选择“用于 Codex” ──> 写入 Grok OAuth CredentialRef
        │                            │
        │                            └─> 启用 Grok 模型并刷新模型目录
        │
系统仅有 ~/.grok/auth.json
        └─ “导入本机 Grok CLI” ──> 选择“用于 Codex”
```

只有用户主动点击“登录新账号”才会启动浏览器 OAuth。

## 9. 生命周期、迁移与兼容

### 9.1 从 Codex Router 迁移

检测到旧 Router 后显示迁移向导：

1. 读取其非敏感 Provider/模型启用状态；
2. 对 Grok：优先匹配 Cockpit 现有账号，再扫描/导入本机 Grok CLI 会话；绝不读取或显示 Router token；
3. 对 API Provider：要求用户在 Cockpit 重新确认密钥，或仅迁移模型与 URL；
4. 创建 Profile 备份并启动统一网关；
5. 运行官方模型、Grok 模型和配置恢复预检；
6. 验证成功后停用旧 Router，保留其安装目录直到用户确认删除。

迁移失败时复用第 5.2 节状态机：仅当当前文件仍匹配 Cockpit 受管 hash 时，才自动恢复旧 Router 快照；存在用户修改时进入冲突界面，绝不自动全量覆盖。无论何种失败，都停止新网关并删除其受管凭据引用，不留下半接管状态。

### 9.2 与现有 Codex API Service 的关系

统一网关是现有 `cockpit-cliproxy` 的下一代配置模式，不新增第二个主路由端口。迁移完成后，`Codex API Service` 与“外部模型”共用同一服务实例、模型注册表、日志和健康检测。

在过渡版本中，旧 API Service、旧 Codex Router、统一网关三者互斥；UI 必须显示当前拥有主路由的组件，并在切换前说明恢复/停用动作。

## 10. 安全与隐私要求

- refresh token 只在 Rust 安全存储与内存出现；前端 DTO 永远为空 token。
- API Key 使用现有安全存储；sidecar 仅获得执行某次请求所需的最小凭据。
- Broker、sidecar、日志与诊断全链路红线脱敏；不得记录 `Authorization`、Cookie、refresh token、API Key 或完整 loopback capability URL。
- 配置与凭据写入使用原子替换、`0600/0700` 权限、锁和崩溃恢复。
- 外部 Provider 的请求日志仅保存模型、Provider、账号 ID（不含邮箱可选）、时间、状态、延迟、token 统计和脱敏错误。
- Gateway 默认绑定 `127.0.0.1`；LAN 监听需要显式用户确认、独立 API Key、风险提示，且不开放 Credential Broker。

## 11. 错误处理与诊断

错误必须带来源和下一步，而不是“安装失败”或“请求失败”。至少区分：

- 官方 Codex 认证无效；
- 统一网关未运行/端口冲突；
- 配置所有权不匹配；
- Grok 账号未授权、token 刷新失败、额度不足、模型不可用；
- Provider 密钥无效、模型不存在、协议能力不兼容、上游限流；
- 目录更新失败但旧目录仍可用。

每个错误可从 UI 直接执行一次安全动作：刷新、重新授权、切换备用账号、重新启动网关、恢复官方模式或导出脱敏诊断包。

## 12. 实施工作包

| 工作包 | 主要改动 | 完成条件 |
| --- | --- | --- |
| A. 统一领域模型 | Rust/TS `UnifiedModelProvider`、模型注册表、迁移 | 序列化向后兼容、导入导出、单测 |
| B. 配置接管 | `codex_account`、`codex_local_access` | 官方 auth 保留、原子备份/恢复、所有权校验 |
| C. Credential Broker | 新 Rust 模块、sidecar client | Grok refresh token 不进入 sidecar 持久化文件 |
| D. Grok OAuth Adapter | `cockpit-cliproxy` Go executor | Responses/SSE/tools/重试/池化全覆盖 |
| E. Provider Adapters | OpenAI-compatible、xAI API | 各协议的契约与集成测试 |
| F. UI | Codex Provider、Grok 账号选择、诊断、迁移向导 | 无重复 OAuth、选择不改默认 Grok CLI |
| G. 迁移与删除旧路径 | Router 检测/迁移/停用 | 迁移失败可回滚，旧 Router 可安全移除 |
| H. 发布质量 | CI、跨平台打包、文档 | 干净机器无需外部 Router 依赖 |

## 13. 验收矩阵

必须在全新用户目录和已安装旧 Router 两类环境中验证：

1. 官方 Codex 登录后开启统一网关，官方模型仍可用，登录不变；
2. 从 Cockpit Grok CLI 账号列表选择现有 OAuth 账号，全程无浏览器 OAuth；
3. 从 `~/.grok/auth.json` 导入已有账号后可选择；
4. 启用 Grok 4.5/4.6 后，模型可见、文本/流式/tool call 正常；
5. Grok token 过期自动刷新；刷新失效后只禁用该账号并提示重新授权；
6. 多 Grok 账号下，429/额度耗尽/401 正确切换且保持会话规则；
7. 官方模型绝不被路由到第三方；第三方模型绝不消耗官方请求；
8. API Provider 的模型发现、手工模型、流式和能力拒绝正确；
9. 启用、停用、崩溃、升级失败、端口冲突后能恢复官方配置；
10. macOS arm64/x64、Windows、Linux 安装后均不要求 Git/Node/Python/uv；
11. 旧 Codex Router 迁移成功与失败回滚都经过验证；
12. 静态扫描、日志检查和导出诊断中不存在 token、API Key 或完整 capability URL；
13. 导入后刷新 Grok 账号，外部 Grok CLI 仍能使用同一账号且未被切换为其他账号；
14. 用户在统一网关激活期间手动修改 `config.toml` 时，停用流程展示冲突且不会直接覆盖修改。

## 14. 代码落点

- `sidecars/cockpit-cliproxy/main.go`：统一 HTTP Surface、Grok OAuth Executor、Provider Adapter、健康/用量事件。
- `src-tauri/src/modules/codex_local_access.rs`：统一网关生命周期、配置投影、sidecar 配置与恢复。
- `src-tauri/src/modules/codex_account.rs`：官方账号配置投影与受管路由保留规则；不读取或复制官方令牌。
- `src-tauri/src/modules/grok_account.rs`：账号引用、可用 access token 的内部接口、独立 profile 语义。
- `src-tauri/src/modules/grok_oauth.rs`：现有 Device OAuth 和 refresh token 逻辑，保持为唯一 OAuth 所有者。
- `src-tauri/src/commands/codex.rs` / `src/services/` / `src/types/`：统一网关命令与类型契约。
- `src/components/codex/`：Provider 管理、Grok 账号选择、模型目录、诊断和迁移 UI。

## 15. 发布门槛

此方案完成的定义不是“能看到 Grok 模型”，而是：普通用户安装 Cockpit 后，无需额外运行时；选已有 Grok CLI 账号后无需重复 OAuth；官方 Codex 登录保持不变；官方模型、Grok OAuth、第三方 API 模型都可稳定选择、调用、诊断、恢复和升级。
