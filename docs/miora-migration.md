# Miora 管理后台与渠道插件迁移

本页描述 Miora 当前实现。README 下半部分保留的是上游历史文档，其中“无前端”、外部控制台和默认管理员 Key 的说明不再适用于本版本。

Codex 会话／缓存、OAuth 完整迁移、Claude 无签名历史、图片与 DashScope 的实现及未验收范围见 [迁移兼容说明](migration-compatibility.md)。上传 ZIP 即托管的插件流程见 [Python SDK](../sdk/python/README.md)。

## 本轮变化

- 首页使用本地登录与管理后台，不再跳转到 `uni-api-web.pages.dev`。
- 浏览器管理员账号和模型调用 API Key 是两个独立认证域。默认用户名是 `admin`，没有默认密码。
- 渠道插件是供应商扩展单位：配置 schema、敏感字段、认证、模型发现、请求执行和 JSON/SSE 响应归插件；核心保留配置、调度、超时、重试和观测。
- 独立 HTTP 服务插件可在线注册、启停、更新和回滚。请求入口固定注册表快照，流体持有租约，旧请求不会因管理更新而切换版本。
- 请求头拦截器有单独的注册表与有序执行链，不代替渠道插件。
- 后台使用内嵌 HTML/CSS/ES modules 与本地 SVG 图标，不依赖前端构建服务、图标字体、远程图标 CDN 或 emoji。

## 渠道与模型配置

新建渠道默认提供七类核心内置入口：**OpenAI 兼容 (`openai`)、Anthropic Claude (`claude`)、Google Gemini (`gemini`)、Codex (`codex`)、Antigravity 反重力 (`antigravity`)、OpenRouter (`openrouter`) 和 Azure OpenAI (`azure`)**。DeepSeek、xAI、SiliconFlow 等兼容 OpenAI 的服务使用“OpenAI 兼容”，填写对应的接口地址即可。其他历史适配不再占用默认新建列表；已有渠道配置仍可使用和编辑，独立 HTTP/SSE 服务插件仍可在“插件与拦截器”页面另外上传 `.miora.json` 注册。

添加渠道时，先选择渠道类型并填写连接信息，再点击“获取模型”在线拉取上游 `/v1/models` 并勾选需要的模型。上游不支持模型目录时，可直接输入任意模型名称手动添加（无模型名硬编码限制）。对外别名在选中模型的行内填写，不需要编辑模型映射 JSON。获取模型不会保存渠道，也不会调用模型生成接口；只有点击保存才更新配置。已有渠道可复用已保存的凭据。

### 三大平台 OAuth PKCE 授权与自动续期

对于 **Codex (`codex`)**、**Anthropic Claude (`claude`)** 与 **Antigravity 反重力 (`antigravity`)** 渠道，保存渠道基础信息后可在渠道列表点击 **「账号池」->「OAuth 登录」**：

- **本机自动回调**：Miora 会在后台启动临时本地回调监听（Codex 监听 `127.0.0.1:1455`，Antigravity 监听 `127.0.0.1:8085`），前端弹窗每 1.5 秒自动轮询授权状态，浏览器完成登录后自动入库。
- **远程 / Docker 手动粘贴**：当网关部署在远程服务器或容器内时，浏览器完成登录跳转到 `localhost` 回调页后，直接复制浏览器地址栏的完整回调 URL（或 Claude 的 `code#state` 授权码）粘贴到弹窗第 3 步输入框即可完成兑换。
- **自动刷新与持久化**：运行时在到期前自动用 `refresh_token` 续期，刷新后的完整凭据原子写入现有 OAuth 凭据存储，不把短期 AT 当作长期迁移，也不需要每次改写渠道引用。

### 自选参数与全部模型并发批量测活

Miora 支持在无需配置网关调用 Key 的情况下，由管理员直接向渠道上游发起真实测活：

- **三处直连测活**：添加/编辑渠道时的草稿模型行内测活、渠道「账号池」单 Key 测活、渠道列表「测试」与顶部「批量测试模型」弹窗及独立 `#testing` 页面。
- **自定义测试参数**：支持自选测试提示词（默认 `hi`）、`temperature`（预设或 `0.0–2.0` 自定义输入）、`top_p`、`max_tokens`、`reasoning_effort`（`low`/`medium`/`high`）、协议端点（`/v1/chat/completions` 或 `/v1/responses`）以及并发数（`1/3/5/10` 并发）。
- **批量筛选与清理**：支持单渠道或跨渠道全部模型并发批量测试、按成功/失败状态筛选、一键重测失败项，以及在渠道编辑器内**一键移除测活失败的模型**。

## 渠道参数与离线请求预览

渠道编辑器按“基础、模型、参数与请求修饰”分开。参数页由当前协议能力提供可用字段，普通表单与高级 JSON 共用 `preferences.parameter_policy`。继承、显式设置、显式删除是不同操作；零值、false、空字符串和 null 不会被当作未设置。没有策略的旧渠道保持原有行为。

预览复用实际请求构造器，不调用生成接口、不保存草稿。Chat 与原生 Responses 已用本地上游比对预览和真实正文；服务插件预览显示的是宿主执行信封，不能据此声称已看到插件内部供应商请求。保存走原有版本冲突检查并热生效，不因点击保存重启进程。具体覆盖顺序、协议限制、敏感请求头和来源报告见 [参数策略](parameter-policy.md)。

参考行为说明为 Zoaholic 的公开 [README](https://github.com/HCPTangHY/Zoaholic)、[插件说明](https://github.com/HCPTangHY/Zoaholic/blob/main/docs/plugin-development.md) 和 [OAuth 管理说明](https://github.com/HCPTangHY/Zoaholic/blob/main/docs/oauth-credential-management.md)。Miora 的三态和覆盖规则是独立设计，不声称参考项目使用同一契约；未复制这些文档、代码或组件。

## 首次启动

需要 Rust 1.84 或更新的兼容工具链，以及本机 C 编译器（SQLite、ring 和 zstd 的构建需要）。生产部署仍可使用仓库 Dockerfile。

```sh
cargo build --locked
python scripts/hash-admin-password.py
```

密码工具交互读取并确认密码，输出带随机盐的 PBKDF2-SHA256 哈希。将输出设置为 `MIORA_ADMIN_PASSWORD_HASH`；不要把密码写进命令行历史或提交进配置仓库。也支持从部署平台的 Secret 注入 `MIORA_ADMIN_PASSWORD`，长度至少 12 个 UTF-8 字节。

本地 HTTP 调试示例（PowerShell；哈希值由上一步生成）：

```powershell
$env:MIORA_ADMIN_PASSWORD_HASH = Read-Host '粘贴生成的管理员密码哈希'
$env:MIORA_ADMIN_SECURE_COOKIE = 'false'
$env:MIORA_ADMIN_ORIGIN = 'http://127.0.0.1:8000'
$env:MIORA_BOOTSTRAP_EMPTY_CONFIG = 'true'
$env:UNI_API_CONFIG_PATH = '.miora/api.yaml'
$env:RUST_RESPONSES_CONFIG_SNAPSHOT_PATH = '.miora/snapshot.json'
$env:UNI_API_SHARED_MEMORY_RESERVATION_PATH = '.miora/memory-ledger'
$env:RUST_REQUEST_SPOOL_DIRECTORY = '.miora/spool'
$env:MIORA_PLUGIN_STORE_DIR = '.miora/plugins'
$env:DISABLE_DATABASE = 'true'
.\target\debug\uni-api-front.exe
```

访问 `http://127.0.0.1:8000/login`，使用 `admin` 和设置的密码登录。`MIORA_BOOTSTRAP_EMPTY_CONFIG=true` 只在配置文件不存在时创建空的渠道和 Key 列表，要求已配置有效管理员凭据，不能和 `CONFIG_URL` 同时使用，不会生成默认调用 Key。现有文件永不被引导流程覆盖。通过后台新增渠道和 API Key 后才能调用模型。

正式部署需要 HTTPS，保持默认 `MIORA_ADMIN_SECURE_COOKIE=true` 并设置准确的 `MIORA_ADMIN_ORIGIN=https://your-host`。使用反向代理时按 [管理员说明](admin.md) 配置受信代理，不能无条件相信客户端提交的 `X-Forwarded-For`。关闭本地 HTTP 调试开关和引导开关。

## 原有配置迁移

原有 `providers`、模型别名、`engine` 及公共 `/v1/*` 路径仍受支持。包名 `uni-api-native` 和二进制名 `uni-api-front` 暂未改名，避免破坏部署脚本。

旧版“第一个 Key”、唯一 Key、`admin-xxx` 角色或根据 token 内容推断角色的隐式管理员规则已移除。确需继续使用旧管理 API 的调用 Key，必须明确声明：

```yaml
api_keys:
  - api: ${MIORA_LEGACY_ADMIN_KEY}
    role: admin
    model: [all]
```

这仍不能用于登录新的浏览器后台。普通模型调用 Key 保持普通角色；不需要给所有 Key 加上 `admin`。管理员后台中新增 Key 默认 `user`，密钥仅创建成功时展示一次，列表不返回完整凭据。

配置写入沿用源文件元数据 revision 做比较并交换；进程内发布者、旧管理写接口和文件 watcher 共用事务锁。沿用必要的配置编译边界，不新增哈希验证。部署只支持单写实例：不要让多个进程、外部编辑器或自动化发布器同时写同一配置或插件存储目录。

## 插件升级的部署约束

详见 [渠道插件开发与协议](channel-plugins.md)。独立服务必须为每个版本使用独立端点/进程；先启动候选版本，再注册并切换，确认旧版本不再有在途引用后才停止旧服务。核心无法防止部署者手动杀掉仍承载请求的旧进程。

注册表保存敏感配置，必须限制目录访问；本轮不创建额外备份。控制通道凭据与上游供应商凭据不可混用。插件是受信服务／脚本，不是任意不可信代码的沙箱。独立服务的非本地地址需明确 HTTPS origin 白名单；托管脚本由宿主自动分配本地端点，不要求手动开放回环策略。

## 当前边界

- 外部插件协议 v1 处理 JSON 请求与 JSON/SSE 响应，不支持 multipart 上传；原有内置渠道的 multipart 能力保留。
- 内置 Rust 适配器随程序发布，动态热更新支持托管 Python 和独立服务插件。现有内置协议转换与原生 Responses 优化执行路径继续复用，不表示每个历史供应商都已拆成可独立部署的软件包。
- 当前拦截器实现是可验证的声明式请求头规则，不提供任意脚本执行。
- 可写本地配置源的系统设置确认保存后持久化；远程只读源不冒充已保存。渠道、调用 Key、插件及其启用状态继续使用既有持久化目录。
- 请求日志已独立提供四阶段的请求头/正文捕获、留存设置、详情下载及两步清理；默认只留摘要，不保存原始内容。用量汇总和管理员审计单独保留，不冒充请求明细；清理新日志不删除旧统计。日志设置持久化，不受其他运行时设置的重启语义影响。参见 [请求日志、留存与清理](request-logs.md)。
- Windows 本地验证补齐了共享资源文件锁和磁盘容量采样；生产 Linux/Docker 的最终部署验证仍应在目标环境执行。

## 回归入口

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --locked
python scripts/verify-http.py target/debug/uni-api-front
```

Windows 将二进制路径改为 `target/debug/uni-api-front.exe`。HTTP 回归均使用临时目录、本地模拟服务及测试凭据，不访问真实付费上游。页面测试与截图方法见 [管理界面说明](admin-ui.md)。

本地 GNU 工具链的日志轮定向验证（不是每次都执行旧全套）：

```powershell
cargo test --locked --offline --target x86_64-pc-windows-gnu request_logs --lib -- --test-threads=1
python tests/http/verify_request_logs.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
python tests/http/verify_request_log_routes.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
python tests/ops/test_local_preview_logging.py -v
python tests/ops/verify_local_preview_supervisor.py target/x86_64-pc-windows-gnu/debug/uni-api-front.exe
```

本地预览由 `scripts/local_preview.py` 管理 stdout/stderr 的有界轮转，并通过独立日志 SQLite 的设置表约每秒刷新限制（静默时也生效）。`.miora/preview/start.py` 作为后台启动入口；它保留本地配置与旧 `stats.db`，设置 `MIORA_RUNTIME_LOG_MANAGED=1`，不控制生产容器或 systemd 日志。

