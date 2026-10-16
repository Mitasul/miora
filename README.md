<p align="center"><img src="assets/branding/miora-logo.jpg" alt="Miora logo" width="240"></p>

<h1 align="center">Miora</h1>

<p align="center">
  <b>纯 Rust 单二进制高性能 AI API 网关与管理控制台</b><br>
  <a href="./README.md">简体中文</a> · <a href="./README_CN.md">中文详版</a> · <a href="docs/channel-plugins.md">插件协议 (v1)</a> · <a href="docs/miora-migration.md">迁移与部署</a>
</p>

---

## 简介

**Miora** 是一个纯 Rust 编写的单二进制 AI API 聚合网关与可视化管理平台。  
它将多种上游大模型服务（OpenAI、Anthropic Claude、Google Gemini、OpenAI Codex、Antigravity 反重力、OpenRouter、Azure、DeepSeek、xAI 等）统一转换为标准 `/v1/chat/completions`、`/v1/responses`、`/v1/messages`、`/v1/embeddings` 等接口，内置现代 Web 管理控制台（`/login` 与 `/admin`），支持多渠道加权/轮询负载均衡、自动冷却、OAuth PKCE 授权与自动续期、可热加载的外部 HTTP/SSE 渠道插件以及全量模型并发批量测活。

---

## 核心特性

### 1. 单二进制内嵌管理控制台（`/login` & `/admin`）
- **零外部前端依赖**：所有管理台 HTML/CSS/ES Modules 与 114 个 AI 品牌本地 SVG 图标直接编译进单个可执行文件，无需额外部署 Nginx 或 Node.js 前端容器。
- **独立管理员安全域**：管理员登录与模型调用 `api_keys` 严格分离，采用 PBKDF2-SHA256 密码派生、CSRF Token 校验与默认 3 天会话有效期（`MIORA_ADMIN_SESSION_TTL_SECONDS=259200`）。

### 2. 三大平台 OAuth 2.0 PKCE 授权与无感续期
- 原生支持 **OpenAI Codex**、**Anthropic Claude**、**Antigravity（Google Cloud Code Assist `v1internal`）** 的 OAuth 2.0 PKCE (`S256`) 登录。
- **本机自动监听 + 远程手动粘贴双模式**：
  - 本机运行时自动监听回调端口（Codex `127.0.0.1:1455` / Antigravity `127.0.0.1:8085`），网页端实时轮询自动入库；
  - 远程服务器或 Docker 容器部署时，复制浏览器跳转后的完整回调 URL（或 Claude 的 `code#state`）粘贴回后台弹窗即可完成绑定。
- **自动刷新与持久化**：运行时在 Token 到期前 60 秒自动使用 `refresh_token` 续期，并原子回写 `api.yaml`。

### 3. 自选参数与全部模型并发批量测活
- **全场景直连测活**：支持新建/编辑渠道时免保存直连草稿测活、渠道「账号池」单 Key 测活、渠道列表单渠道/跨渠道批量测试弹窗、以及独立 `#testing` 测试台。
- **丰富自定义测试参数**：
  - **测试提示词 (`prompt`)**：默认 `hi`，可任意修改；
  - **温度 (`temperature`)**：支持预设档位（默认不传 / `0` / `0.2` / `0.5` / `0.7` / `1.0`）或选择「自定义…」输入 `0.0–2.0` 任意数值；
  - **Top P (`top_p`)**、**最大输出 Token (`max_tokens`)**、**思考强度 (`reasoning_effort`: `low` / `medium` / `high`)**；
  - **协议端点切换**：`/v1/chat/completions` 或 `/v1/responses`；
  - **并发控制**：支持 `1 / 3 / 5 / 10` 并发批量测活。
- **结果筛选与一键清理**：支持按全部/成功/失败/未测试筛选、一键重测失败项，以及在渠道编辑器内**一键移除测活失败的模型**。

### 4. 内置核心适配器 + 动态外部渠道插件（`miora.plugin.v1`）
- **7 大核心内置适配器**：
  - `openai`（OpenAI 兼容，适用 DeepSeek / xAI / SiliconFlow / Groq 等）
  - `claude`（Anthropic `/v1/messages`，支持 API Key 与 `sk-ant-oat...` OAuth 自动 `anthropic-beta` 头）
  - `gemini`（Google Gemini 原生接口，支持 **Gemini 4 / 3.8** 自动过滤禁用采样参数与 `thinkingLevel` 映射）
  - `codex`（OpenAI Codex Responses 接口 + OAuth PKCE）
  - `antigravity`（Antigravity 反重力 `v1internal:generateContent` + Google OAuth + 自动获取 `project_id`）
  - `openrouter`（OpenRouter 路由与模型发现）
  - `azure`（Azure OpenAI 部署适配）
- **外部 HTTP/SSE 渠道插件**：在后台「插件与拦截器」上传 `.miora.json` 描述文件即可在线注册、启停、升级与回滚独立插件服务，请求固定版本快照，热更新不中断在途流式响应。

### 5. 三态渠道参数策略、可视分组与四阶段请求日志
- **三态参数策略 (`parameter_policy`)**：按渠道与模型作用域配置参数继承、显式覆写或显式删除，支持零成本离线请求体预览。
- **可视分组标签 (`.group-chip`)**：渠道与访问密钥均采用点击式标签选择器管理分组（内置 `default` / `vip` / `coding` / `fast` 并支持自定义分组）。
- **四阶段请求日志与计费统计**：独立捕获客户端入站、路由重写、上游出站、上游响应四阶段数据，支持按渠道/按模型配置输入输出 Token 单价。

---

## Docker 快速部署（推荐：直接拉取预编译镜像，无需编译）

每次推送到 `main` 后，GitHub Actions 会自动编译并发布预构建镜像 `ghcr.io/mitasul/miora:latest`，服务器上**无需本地编译 Rust**，几秒即可启动：

### 1. 使用 `docker compose` 一键秒级启动

```bash
git clone https://github.com/Mitasul/miora.git
cd miora

# 1. 创建持久化数据目录（存放 api.yaml、SQLite 数据库与插件）
mkdir -p miora_data

# 2. 若从 Zoaholic / 旧环境迁移，直接将已有 api.yaml 放入 miora_data：
# cp /path/to/your/api.yaml ./miora_data/api.yaml

# 3. 编辑 docker-compose.yml 中的 MIORA_ADMIN_PASSWORD（至少 12 位），然后直接拉镜像启动：
docker compose up -d
```

启动后访问 `http://服务器IP:8000/login`，使用用户名 `admin` 与设置的密码登录后台。  
*(如需在本地从源码自建镜像，可运行 `docker build -t ghcr.io/mitasul/miora:latest .`)*

### 2. 使用 `docker run` 直接启动

```bash
docker run -d \
  --name miora \
  --restart unless-stopped \
  -p 8000:8000 \
  -e MIORA_ADMIN_USERNAME=admin \
  -e MIORA_ADMIN_PASSWORD=your-strong-password-12chars \
  -e MIORA_ADMIN_SECURE_COOKIE=false \
  -e MIORA_ADMIN_SESSION_TTL_SECONDS=259200 \
  -e MIORA_BOOTSTRAP_EMPTY_CONFIG=true \
  -e UNI_API_CONFIG_PATH=/home/data/api.yaml \
  -e DB_PATH=/home/data/stats.db \
  -e MIORA_PLUGIN_STORE_DIR=/home/data/plugins \
  -v $(pwd)/miora_data:/home/data \
  ghcr.io/mitasul/miora:latest
```

---

## 从 Zoaholic 迁移说明

1. **直接复用 `api.yaml`**：将原有的 `api.yaml` 放入挂载目录 `/home/data/api.yaml`，原有的 `providers`、`api_keys`、模型映射（`upstream: public_alias`）、分组（`groups`）、权重与冷却配置原样生效。
2. **动态模型拉取（无硬编码列表）**：在渠道编辑页点击 **「获取模型」** 即可实时请求上游 `/v1/models`，也可手动输入任何新发布的模型 ID。
3. **OAuth 账号池迁移**：在「渠道与模型」点击对应渠道的 **「账号池」->「OAuth 登录」**，支持单渠道挂载多个 Codex / Claude / Antigravity OAuth 账号并自动轮询与续期。

---

## 外部渠道插件开发简述（`.miora.json`）

在后台 **「插件与拦截器」** 页面上传单个 `.miora.json` 服务描述文件即可安装自定义渠道插件：

```json
{
  "format": "miora.plugin.v1",
  "manifest": {
    "id": "my-provider",
    "name": "My Provider",
    "version": "1.0.0",
    "api_version": 1,
    "capabilities": {
      "endpoints": ["/v1/chat/completions", "/v1/responses"],
      "streaming": true,
      "model_discovery": true
    },
    "config_schema": [
      { "name": "api_key", "label": "API Key", "field_type": "secret", "required": true }
    ]
  },
  "endpoint": "https://plugin.internal.example.com"
}
```

完整插件协议与示例服务详见 **[docs/channel-plugins.md](docs/channel-plugins.md)** 与 `examples/plugins/`。

---

## 环境变量说明

| 变量名 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `MIORA_ADMIN_USERNAME` | `admin` | 管理后台登录用户名 |
| `MIORA_ADMIN_PASSWORD` | 无（必填其一） | 管理后台明文密码（12–1024 字节，启动时派生 PBKDF2-SHA256） |
| `MIORA_ADMIN_PASSWORD_HASH` | 无（必填其一） | 预生成的 `pbkdf2-sha256$...` 哈希（由 `python scripts/hash-admin-password.py` 生成） |
| `MIORA_ADMIN_SESSION_TTL_SECONDS` | `259200` (3天) | 管理后台登录有效期（`300` – `2592000` 秒，即 5 分钟至 30 天） |
| `MIORA_ADMIN_SECURE_COOKIE` | `true` | 纯 HTTP 访问后台时请设为 `false`；HTTPS 环境保持 `true` |
| `MIORA_ADMIN_ORIGIN` | 自动按 Host 推断 | 显式限制管理后台允许的 Origin（如 `https://api.example.com`） |
| `MIORA_BOOTSTRAP_EMPTY_CONFIG` | `false` | 配置文件不存在时自动创建空 `api.yaml` |
| `UNI_API_CONFIG_PATH` | `./api.yaml` | 渠道与密钥配置文件路径 |
| `DB_PATH` | `./stats.db` | 用量统计与日志 SQLite 数据库路径 |
| `MIORA_PLUGIN_STORE_DIR` | `./plugins` | 外部插件与拦截器存储目录 |
| `ANTIGRAVITY_OAUTH_CLIENT_SECRET` | 内置默认值 | 可选自定义覆盖 Antigravity OAuth Client Secret |

---

## 文档目录

- [部署与迁移指南](docs/miora-migration.md)
- [管理员安全与认证](docs/admin.md)
- [渠道插件协议 (Service Protocol v1)](docs/channel-plugins.md)
- [管理控制台 UI 与静态契约](docs/admin-ui.md)
- [渠道三态参数策略与离线预览](docs/parameter-policy.md)
- [四阶段请求日志留存与清理](docs/request-logs.md)
- [网关功能清单](docs/gateway-features.md)
- [性能验证报告](docs/performance.md)

## 致谢与许可证

- 本项目基于 [uni-api](https://github.com/yym68686/uni-api) 构建，并在演进过程中借鉴了 [Zoaholic](https://github.com/HCPTangHY/Zoaholic) 的部分功能与交互思路（如 OAuth 凭据管理、渠道插件化扩展、参数策略与模型批量测活体验）。
- 遵循 **Apache-2.0 License** 开源许可证（保留上游历史声明与依赖归属，详见 [docs/dependency-licenses.md](docs/dependency-licenses.md)）。
