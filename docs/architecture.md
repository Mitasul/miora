# Miora Rust 架构与目录

## 当前扩展与控制台

本版本在下述上游网关基线上增加：

| 路径 | 职责 |
| --- | --- |
| `src/plugins/` | 渠道插件 manifest、内置适配器注册、独立服务协议、注册表版本、流式租约、请求拦截器和管理接口。 |
| `src/admin/` | 独立管理员账户、会话、CSRF、管理视图、渠道和 API Key 持久化管理。 |
| `src/api/admin_ui.rs` | 本地登录和控制台静态资源白名单、CSP、同源安全边界。 |
| `assets/admin/` | 无 emoji、本地 SVG 图标的嵌入式管理界面。 |
| `examples/plugins/` | 不调用真实供应商的独立插件协议示例。 |

推理请求入口固定插件版本快照；注册表更新不修改该快照，响应流持有租约直到 EOF 或客户端取消。控制平面先校验配置/端点/候选健康，再按 revision 原子发布；禁用停止新请求分配，旧版本继续服务在途请求。

插件负责供应商语义，核心仍负责调度、重试、资源治理和日志。原有 Rust 适配器已移入内置插件实现，通用协议转换与原生 Responses 优化链路保留；新增独立服务插件不新增核心供应商分支。拦截器是单独的声明式请求头扩展。

管理员浏览器会话不等于模型调用 Key；管理路由在公共 API CORS 之前处理。原有 Key 管理权限要求显式 `role: admin`，不会根据 token 内容、位置或数量提权。配置源的 legacy 写操作、后台 CAS 和 watcher 在进程内统一串行发布。

当前部署、协议和限制以 [迁移指南](miora-migration.md)、[管理安全](admin.md)、[渠道插件](channel-plugins.md) 为准。

## 上游基线说明

以下保留原网关模块职责与历史迁移背景，其中外部 `uni-api-web` 边界和“未引入新配置协议”的描述仅指上游目录重构，不代表当前内置控制台与插件实现。

uni-api 是独立的模型路由网关。仓库根目录是单个 Rust crate，包名保留
`uni-api-native`，可执行文件保留 `uni-api-front`。Python 仅用于离线 HTTP
回归；历史 Python 服务设计位于 `docs/archive/`。

## 源码地图

| 路径 | 职责 |
| --- | --- |
| `src/main.rs` | 启动 Tokio 并调用库入口。 |
| `src/app.rs` | 配置、依赖和 HTTP 服务的组装，启动恢复及优雅退出。 |
| `src/api/` | 公共 API、管理 API、认证入口、HTTP 分发和 CORS。 |
| `src/config/` | 外部配置读取、模型发现、配置编译与快照数据类型。 |
| `src/control/` | 渠道意图、设置、变更校验、版本冲突检查、恢复和原子应用。 |
| `src/routing/` | 身份与模型权限、目录投影、渠道筛选排序、超时与失败策略。 |
| `src/runtime/` | 应用上下文、当前快照、调度游标、限流冷却、幂等和资源状态。 |
| `src/upstream/` | HTTP 客户端池、请求准备与执行、重试及 hedging。 |
| `src/providers/` | 上游请求构造、地址与鉴权规则、Provider 特例和模型目录。 |
| `src/protocols/` | 协议内容转换、SSE framing、流事件解析和 Responses 修复规则。 |
| `src/transport/` | HTTP 头过滤、请求/响应体读取、解压、multipart 与 spool。 |
| `src/observability/` | 请求上下文、dispatch/stream 时间、用量、指标及事实生成。 |
| `src/storage/` | 可选数据库、请求事实持久化及 S3 导出。 |
| `assets/` | 编译期内置模型目录和静态图标，不放生产渠道配置。 |
| `tests/http/` | 真实进程与隔离本地上游的 Python 回归。 |
| `tests/fixtures/` | 配置、协议和流的固定样例；`manual/` 是历史手工样例。 |
| `tests/support/` | 手工 mock server。 |
| `scripts/` | 开发与验证入口。 |

单元测试仍使用 Rust 原生 `#[cfg(test)]` 模块。较大的测试集移入所属模块
旁边的 `tests.rs`，可以访问内部实现而不扩大对外库 API。

## 配置与状态

配置意图来自外部 YAML/`CONFIG_URL`，以及控制台保存并下发的渠道覆盖。
`config/source.rs` 处理读取、发布和现有配置更新接口；
`config/compiler.rs` 处理配置编译；`config/snapshot.rs` 定义快照和渠道类型。
`control/` 负责完整候选校验、revision 检查、启动恢复和覆盖的原子应用。

`runtime/state.rs` 的 `GatewayRuntime` 持有当前快照及渠道覆盖。
短期调度事实集中在 `runtime/scheduling.rs` 的 `SchedulingState`，包括 key
和渠道冷却、失败历史、限流窗口及调度游标。它们不写回配置意图。
为保持行为，Provider 仍引用原有共享 key 游标，各状态的 Arc、锁和作用域不变。

启动顺序仍为：加载并编译基础配置、初始化可选持久化、读取快照、恢复保留
渠道设置、最后监听 HTTP。保留原有恢复凭据、启动 receipt 和 unchanged 检查；
校验失败不把未完成的候选覆盖到服务状态。

目录调整没有引入新的 artifact 签名、配置协议或跨实例状态存储。
代码镜像不携带生产配置；配置恢复与旧有效配置的保护依靠既有协议及部署机制，
不能仅凭源码分目录认定这些保证已经成立。数据库仍可通过 `DISABLE_DATABASE`
关闭，不因重构而成为生产必需依赖。

## 请求执行与协议

HTTP 入口在 `api/`。共享身份、权限及路由能力位于 `routing/`，不归属于
Responses 端点。普通请求执行在 `upstream/generic.rs`；Responses 的执行和
逐次尝试状态在 `upstream/responses.rs` 及其 `route.rs`、`prepare.rs`。
两种执行路径的重试、提交和取消语义保持原样，没有在此次整理中强行合并。

`providers/request.rs` 提供统一的 wire request 构造入口。管理预览和真实
执行都使用它；预览不发送模型请求。Provider 特有鉴权、URL 和模型行为按
上游归属组织；通用协议转换与 SSE 处理位于 `protocols/`。
Provider 和协议模块不依赖 HTTP handler 或请求重试执行器。

`upstream/client.rs` 保留现有连接池的 key、连接参数及复用策略。
`runtime/context.rs` 组装应用状态；`app.rs` 只负责进程启动生命周期。

## 系统边界

- uni-api 执行渠道配置、选路、重试与冷却，并生成请求事实。
- uni-api-web 控制台负责管理流程、配置意图的持久保存和分析。
- 0-0 的账户、余额和账务主事实，以及 OAIX 的账号池，仍属于各自系统。
- S3 请求事实与可选数据库实现留在网关；分析缓存和控制台业务不搬入网关。

## 开发、构建与回归

从仓库根目录执行：

```bash
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python3 scripts/verify-http.py
docker build -t uni-api:local .
```

HTTP runner 默认运行所有 `tests/http/verify_*.py`，也可传入指定二进制：

```bash
python3 scripts/verify-http.py /absolute/path/to/uni-api-front
python3 tests/http/verify_control_restore.py target/debug/uni-api-front
```

这些测试使用本地 mock 和临时配置，不需要业务 key、真实付费请求或生产数据库。
`.dockerignore` 只允许 Cargo 文件、源码、内置资源和 README 进入构建上下文。
编译期资源统一通过 `CARGO_MANIFEST_DIR` 定位。

## 迁移对应关系

| 原路径或模块 | 当前归属 |
| --- | --- |
| `rust/uni-api-native/Cargo.*`、`.cargo/` | 仓库根目录。 |
| `rust/uni-api-native/src/bin/uni-api-front/` | `src/` 下按职责组织。 |
| `responses_native.rs` | `config/snapshot.rs`、`routing/`、`runtime/`、`upstream/responses/`。 |
| `generic_api.rs` | `api/gateway.rs`、`providers/`、`protocols/`、`upstream/generic.rs`。 |
| `proxy.rs` | `api/handler.rs`、`runtime/context.rs`、`upstream/client.rs`、`transport/`。 |
| `native_api.rs` | `api/platform.rs`。 |
| `channel_controls.rs`、`channel_settings.rs` | `control/channels.rs`、`control/settings.rs`。 |
| `static/`、`uni_api/api/*.json` | `assets/static/`、`assets/codex/`。 |
| `scripts/verify_*.py` | `tests/http/`，统一入口是 `scripts/verify-http.py`。 |
| `test/fixtures/`、`test/runtime_contracts.fixture` | `tests/fixtures/`。 |

二进制名、镜像入口、HTTP 路由、环境变量名、`/home/api.yaml`、快照格式、
恢复协议和事实格式均保留。目录迁移不应与行为改动、依赖升级或生产发布混在一起。
