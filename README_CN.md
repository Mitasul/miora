<p align="center"><img src="assets/branding/miora-logo.jpg" alt="Miora logo" width="240"></p>

<h1 align="center">Miora</h1>

<p align="center">
  <b>纯 Rust 单二进制高性能 AI API 网关与管理控制台</b><br>
  <a href="./README.md">主文档 (README.md)</a> · <a href="docs/channel-plugins.md">插件协议 (v1)</a> · <a href="docs/miora-migration.md">迁移与部署</a> · <a href="docs/parameter-policy.md">参数策略</a>
</p>

---

## 核心架构与功能

- **内嵌现代管理后台（`/login` & `/admin`）**：前端资源编译进单一二进制 `uni-api-front`，管理员认证与推理调用 API Key 完全分离，默认登录会话持续 3 天（`259200` 秒）。
- **三大平台 OAuth PKCE 登录与自动续期**：原生支持 **OpenAI Codex**、**Anthropic Claude**、**Antigravity（反重力）** OAuth 2.0 PKCE (`S256`) 授权（本地端口自动捕获回调 + 远程/Docker 粘贴回调 URL 双模式），运行时到期前自动刷新 Token 并写回 `api.yaml`。
- **自选参数与全模型并发批量测活**：支持自定义 `prompt`、`temperature`、`top_p`、`max_tokens`、`reasoning_effort`、端点与并发数（`1/3/5/10` 并发），支持单渠道/全渠道批量测活、按状态筛选、一键重测失败项与一键移除失败模型。
- **7 大内置适配器 + 动态外部插件 (`miora.plugin.v1`)**：内置 `openai`、`claude`、`gemini`（含 Gemini 4 适配）、`codex`、`antigravity`、`openrouter`、`azure` 适配器，并支持上传 `.miora.json` 在线热注册外部 HTTP/SSE 渠道服务。
- **四阶段独立请求日志与精准用量核算**：客户端入站、路由重写、上游出站、上游响应四阶段捕获与脱敏，支持按渠道/按模型自定义 Token 价格。

---

## `api.yaml` 配置结构参考

除直接通过 `/admin` 网页后台可视化管理所有渠道、OAuth 账号、模型映射、分组与访问密钥外，Miora 也完全兼容直接编辑 `api.yaml`：

```yaml
providers:
  - provider: openai-main
    engine: openai
    base_url: https://api.openai.com/v1/chat/completions
    api:
      - sk-xxxx1
      - sk-xxxx2
    groups:
      - default
      - vip
    model:
      - gpt-5.2
      - claude-sonnet-4-5-20250929: claude-sonnet-4-5

  - provider: gemini-native
    engine: gemini
    base_url: https://generativelanguage.googleapis.com/v1beta
    api: AIzaSyXXXX
    model:
      - gemini-2.5-pro
      - gemini-4

api_keys:
  - api: sk-miora-client-key-xxxx
    role: user
    groups:
      - default
      - vip
    model:
      - all
```

更多详细说明请查阅 **[README.md](./README.md)** 与 **[docs/miora-migration.md](docs/miora-migration.md)**。

---

## 致谢与许可证

- 本项目基于 [uni-api](https://github.com/yym68686/uni-api) 构建，并在演进过程中借鉴了 [Zoaholic](https://github.com/HCPTangHY/Zoaholic) 的部分功能与交互思路（如 OAuth 凭据管理、渠道插件化扩展、参数策略与模型批量测活体验）。
- 遵循 **Apache-2.0 License** 开源许可证（详见 [docs/dependency-licenses.md](docs/dependency-licenses.md)）。
