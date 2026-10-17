# 网关入口、路由与调试

本页记录 Miora 自身的实现与边界，只借鉴真实使用流程，不移植 Zoaholic 的代码或协议。

## Gemini 与 Messages 入口

新增入口：

- `GET /v1beta/models`
- `POST /v1beta/models/{model}:generateContent`
- `POST /v1beta/models/{model}:streamGenerateContent`

Gemini 客户端可以用 `x-goog-api-key` 或 `?key=` 提供 **Miora 网关 Key**；查询串中的 `key` 会在请求日志与追踪建立前移除。已有显式认证头优先。模型列表沿用该 Key 的模型授权和渠道分组，不公开未授权模型。

`/v1/messages` 可通过现有 Chat 适配链转到 OpenAI、Responses、Gemini 等上游；Gemini 入口也可以转到 Chat、Responses、Claude 等内置上游。请求与响应都转换，流式请求输出对应的 Messages 事件或 Gemini SSE，而不是返回另一种协议的正文。

同协议 Claude Messages / Gemini 请求保留原生结构与扩展字段；跨协议才经过转换层。渠道显式参数策略仍会修改匹配字段，因此这不是字节级零拷贝承诺。Gemini 流式上游 URL 使用 `alt=sse`。Vertex Gemini 也可接收原生 Gemini 请求。

跨协议支持文本、内联图片、函数声明与工具往返、常用生成参数。不能表达的文档、`fileData`、内置 Gemini 工具以及音视频内容返回转换错误，不伪造兼容成功。Gemini 跨协议流式目前只支持一个候选；原生 Gemini 不受该转换限制。不同厂商的签名思考状态不能无损互换，也不会伪造签名。

文本与思考逐段输出；交错的并行工具增量在完成时合并为完整工具块，避免生成重叠的 Messages 内容块或残缺 Gemini 函数 JSON。没有终止事件的断流返回错误，不补造成功的结束标记。

## 虚拟模型

入口：后台「渠道」→「虚拟模型」。也可在工作区源配置中编辑：

```yaml
providers:
  - provider: primary
    base_url: https://primary.example/v1
    api: ${PRIMARY_API_KEY}
    model:
      - upstream-model: public-model
      - upstream-alternate: alternate-model
  - provider: backup
    base_url: https://backup.example/v1
    api: ${BACKUP_API_KEY}
    model: [other-model]

virtual_models:
  assistant:
    - provider: primary
      model: public-model
    - provider: backup
      model: other-model
      on_status: [429, 502, 503]
    - virtual: fallback
  fallback:
    - provider: primary
      model: alternate-model
```

`model` 引用渠道对外模型名，不是模型映射左侧的上游 ID。虚拟名不得与现有渠道的对外模型重名。编译配置时展开嵌套引用并拒绝循环和未知目标；停用渠道保留源定义，但不进入运行时路由。

目标按声明顺序尝试，遍历一次，不开启并发抢跑或递归重试。每个虚拟模型的第一个目标必须无条件；`on_status` 只放在具体目标上，匹配前一次失败的 HTTP 状态。通常的可重试错误仍可进入无条件后备；明确匹配的条件目标也可处理指定的其他错误状态。渠道凭据不可用时可跳到后续目标。既有请求重试上限仍生效，最多 100 次物理尝试；首次已经向客户端输出语义内容后不重放请求。

网关 Key 可通过 `assistant`、`<assistant>` 或 `all` 授权虚拟模型；每个目标仍按 Key 与渠道分组交集筛选，不能借虚拟名越权。目标模型的逐模型价格用于该次实际渠道结算，不给后备模型套用主模型单价。

## BYOK

渠道默认关闭 BYOK，在编辑器启用，或配置：

```yaml
preferences:
  allow_byok: true
```

调用时同时提供：

```text
Authorization: Bearer <Miora网关Key>
x-miora-byok-provider: primary
x-miora-byok-api-key: <调用方上游Key>
```

BYOK 不是匿名代理。网关认证、模型规则、分组与渠道启停继续有效；仅支持 API Key 型内置渠道：GPT、OpenRouter、Claude、Gemini、Azure、Azure Databricks，不接管 OAuth、云签名或服务插件凭据。

调用方凭据只存在于本次请求，不写入配置、公共池或公共冷却/失败计数，也不进行公共 Key 轮换；单次 BYOK 最多尝试一次。最终认证头以调用方凭据为准，渠道的自定义认证头不能覆盖它。渠道统计的凭据归类统一为 `BYOK`，不持久化调用方上游 Key。请求用量仍归属已认证的网关 Key。

## 渠道级响应与思考策略

```yaml
preferences:
  response_policy:
    retry_empty: true
  reasoning_history: preserve
  reasoning_suffixes:
    "-high":
      effort: high
      budget_tokens: 2048
    "-thinking":
      effort: high
      budget_tokens: 2048
```

- `retry_empty` 默认关闭；启用后，空成功正文及首个语义输出前的流内软错误作为失败尝试处理，可使用已有重试与后备渠道。拒绝回答/安全拦截不是空响应。已输出内容后不会切渠道重复生成。
- `reasoning_history` 默认 `preserve`，也可选择 `strip` 移除多轮历史中的思考字段/块。不建立服务端历史回填缓存，不推测缺失思考内容。
- 后缀在配置编译时生成模型别名，由既有参数策略映射到实际协议的 effort / token budget。显式请求参数优先。Claude 思考预算与输出上限冲突时返回错误，不暗改客户端上限。

后台渠道编辑器提供这些开关；已有自定义后缀映射在保持开启时保留，不被固定模板覆盖。

## 渠道与凭据管理

渠道列表提供永久启停、选择可见渠道、批量启停和批量分组。批量操作一次校验、一次保存；隐藏行不进入本次选择。关闭渠道不删除配置、凭据或历史统计。

渠道「凭据」面板可以导入 JSON / 文本 Key 列表，逐 Key 永久启停，显示本进程冷却剩余秒数与累计失败次数。关闭面板或隐藏页面停止轮询。源配置保留完整 `api` 列表，以 `disabled_key_positions` 的一基序号记录停用项；整体替换凭据时清除旧序号。

Codex 渠道支持导入 `auth.json` 的 `tokens.account_id` 与 `tokens.refresh_token`，显示缓存/过期状态，并手动执行现有的真实 OAuth 刷新。刷新失败不会显示成功，持久化失败不会被内存缓存掩盖。手动刷新会联系供应商授权服务器；导入本身不会自动测试上游。

Claude 与 Antigravity 的浏览器授权在供应商返回 refresh token 时保存可刷新凭据，自动刷新和后台「刷新授权」使用各自的授权端点。刷新后的 token 继续写入现有 OAuth 凭据存储，重启后可继续使用；Codex 的 `account_id,refresh_token` 格式保持兼容。此前只保存 access token 的旧账号需要重新授权一次，无法从 access token 还原 refresh token。静态 Key 不显示 OAuth 刷新入口。

目前没有 Gemini CLI、Grok 的浏览器 OAuth 授权流程。渠道额度查询全部由插件执行，支持套餐、余额和多个限额窗口，手动刷新并持久保存；不会为普通渠道猜测余额接口。随项目提供 Codex、NovelAI、DeepSeek、Kimi 开放平台和 Kimi Coding Plan 五个独立余额插件。

## 流式对话调试台

后台「对话调试」使用已保存的网关 Key，在服务端发起真实 Chat / Responses 流式推理。支持最多三路模型同屏对比、各自的多轮历史、停止、清空、思考折叠、工具调用展示、用量与 Request ID。

打开页面不会自动测试，不把已保存 Key 返回浏览器，不持久化提示词。工具调用仅展示，不执行工具，也不把尚未完成工具结果的轮次加入聊天历史。模型或 Key 切换时清除对应历史。真实发送可能计费。

## DATABASE_URL 与 PostgreSQL TLS

非空 `DATABASE_URL` 优先于 `DB_TYPE` / `DB_*`：

```text
DATABASE_URL=postgresql://user:password@db.example:5432/miora?sslmode=require
DATABASE_URL=sqlite://./data/stats.db
```

URL 用户名/密码支持百分号编码。PostgreSQL URL 默认 `sslmode=require`，使用 rustls 校验证书链和主机名；显式 `disable` / `prefer` 可用。`verify-full` / `verify-ca` 都采用证书链与主机名校验，不提供跳过证书校验的模式。`prefer` 可以在服务器不支持 TLS 时退回明文，云部署应使用 `require`。私有 CA 用 `DB_SSL_ROOT_CERT` 指向 PEM 文件。

没有 URL 时保持既有 `DB_*` 行为，默认 TLS 为 `disable`，可设置 `DB_SSLMODE`。SQLite URL 仅支持持久文件路径，内存与未支持的查询参数会返回配置错误。数据库禁用时不连接数据库。

## 实现与验收边界

协议层位于 `src/protocols/dialects.rs`，路由展开位于 `src/config/virtual_models.rs`，渠道策略位于 `src/providers/reliability.rs`；管理页面继续使用本地原生 JS，不增加前端框架或每请求插件扫描。

本轮只做离线 Rust 纯单测、前端模块与静态资源检查，不启动/重启网关、不运行浏览器或外部插件、不执行付费推理，也没有 PostgreSQL TLS 实机验收。前端资源嵌入 Rust 二进制，需要后续由用户安排构建部署；保存的启停与配置在下一次正常启动继续生效。

当前证据：针对性 Rust 用例 23 项，连同相关模块回归共 116 项不同用例通过；前端模块 113 项、静态资源合约 14 项通过；Rust 默认目标集的离线编译检查通过。用例包括配置增删持久化、BYOK 不入池/不落库及认证头覆盖、虚拟路由分组和价格、分片工具参数、断流不伪造完成、OAuth 写入失败后的重试，以及数据库 URL 解析。
