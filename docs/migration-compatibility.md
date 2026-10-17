# Zoa → Miora 迁移兼容范围

基于当前 Miora 独立实现，不包含 Zoa GPL 代码或其插件协议。现有 Rust 渠道、别名、调用 Key、分组和持久化源仍是权威数据；本轮不初始化数据目录、不重置配置、不生成配置／数据库备份。不要把“构造器有分支”或离线 fixture 成功当成上游已经兼容。

## Codex 会话与缓存

自动会话选择优先级：非空 `session_id` → 非空 `session-id`（也就是大小写任意的 `Session-Id`）→ 非空 body `prompt_cache_key` → 当前 request ID。大小写不敏感不意味着横线与下划线相同。出站使用 Codex 的 `session_id` 字段。

只有缓存 key 时，直接对其 UTF-8 字节作可逆 URL 字节编码，不计算哈希、不随机生成新会话 ID。同一缓存 key 保持稳定；没有任何会话／缓存标识时才按请求区分。独立请求 ID 不被强制合并为会话 ID。原生 Responses 与 Chat→Responses 均保留 `prompt_cache_key`、`client_metadata`。操作者显式配置的 header 参数策略及拦截器仍是有意修饰，不被自动逻辑悄悄删除。

最终 Codex 请求含历史 `web_search_call` 而没有 web_search／web_search_preview 声明时补声明。原工具缺失、null 或空列表时，同时强制 `tool_choice: none`，即使原值是 auto 或 required；存在其他工具时保留其工具选择。已有搜索声明不重复添加，不删除历史，不主动打开新搜索。相同处理应用于普通 `/responses` 发送的摘要请求；原生 `/responses/compact` 不套用标准 tools 补齐规则。**尚无 Responses Lite 专用协议实现。**

输入、输出、缓存读取／写入取自上游 usage；detail 对象在 Responses→Chat usage 转换中保留。完成事件的 usage 会进入原有日志与统计结算，不把 HTTP 200 当成流成功，不把缺失指标补成“命中为 0”。

### 实际缓存验收（尚未执行）

1. 停止旧实例刷新同一账号，再导入完整 OAuth 账号，固定账号、上游模型及会话／缓存 key。
2. 连续请求使用足够长的相同前缀，观察实际出站 body/header，确认 session 不随各次 request ID 变化。
3. 看最终 `response.completed` 和完整 usage，逐次对比 input/output、cached tokens、缓存未命中及成本统计。日志正文需按现有日志设置明确开启，注意敏感内容。
4. 单次未命中不是失败；缓存是上游行为，不承诺每次命中。重试、排队或改账号后也不能算作同账号连续验收。

## OAuth 迁移与续期

渠道编辑器导入文件和账号池导入都把完整 JSON 交给同一后台导入逻辑，不再只提取一串短期 AT。Codex 接受根对象或 `tokens` 内的 access_token、refresh_token、account_id，以及 Unix 秒／毫秒 expires_at；也可从 JWT 声明取得账号与到期信息。Claude 可导入 `claudeAiOauth` 的 camelCase 字段。JSON 对象数组可批量导入。

- 渠道配置仍保存原有凭据引用格式，如 `account_id,refresh_token`、`oauth:claude,refresh_token`。
- 有 RT 的完整凭据记录保存到**现有** `CODEX_REFRESH_TOKEN_STORE_PATH`，默认 `data/codex_refresh_tokens.json`：AT、最新 RT、expiry、账号 ID、邮箱和套餐信息一起持久化。旧“键 → RT 字符串”文件继续可读。
- 成功刷新后先持久化再更新内存缓存。重启能复用尚有效的 AT，并用保存的最新 RT 续期；配置引用无需随每次轮换改名。当前进程的刷新锁、登录会话和缓存仍只在内存。
- 单独 AT 可以按普通凭据导入，但没有长期续期能力；其额外导入元信息仅在内存，原 AT 仍由渠道配置保存。缺少 expiry 的导入 AT 不被当成永不过期，应通过 RT 续期。
- 不做签名伪造或本地密码学签名验证。Codex 使用明确账号 ID；其他 OAuth 供应商不误带 ChatGPT 账号头。
- 导入 RT 要明确确认旧实例已停止刷新。Miora 不能远程阻止仍在运行的 Zoa：**先停旧实例／解绑该账号，再迁移；不要共享同一轮换 RT 给两个独立网关。** 需要两边并行时，使用独立授权凭据。确认提示不是跨系统锁。
- 损坏或不可读的已存在凭据文件会报错，不默默当空文件覆盖。部署者仍需保护持久化目录 ACL；这里不是加密 token 保险库。

离线测试覆盖完整导入→持久化→新 manager 重载、模拟轮换保存、旧格式读取及文件错误不覆盖。**真实流式、工具调用、账号额度／账号选择、到期后的真实续期尚未验收。**

## Claude 无签名历史

每个渠道独立选择 `preferences.unsigned_thinking`：默认 preserve；设为 strip 仅删除 assistant 历史中 signature 缺失、非字符串或空白的 thinking 块。保留非空 signature 和 redacted_thinking，不伪造签名，也不尝试计算或验证签名。

只有本次确实删除了 thinking、且剩余 content 为空数组或仅有空白 text 块时，才移除该 assistant 消息。原本为空的消息、不认识的内容块、合法非空签名块、redacted_thinking 和工具调用仍保留；再次应用不会继续改写历史。

最终 Claude 请求构造位置应用此策略，覆盖原生 Messages 与 Chat→Claude；Messages 经 Chat 桥接到 Vertex Claude／Bedrock 时也保留原有 thinking/signature 块再应用策略。签名是否被上游接受仍由供应商决定；仅有非空字段不意味着本地证明了签名有效。已有显式 reasoning_history 剥离策略仍按用户设置生效。

## 图片与 DashScope：实现与验收分开

| 场景 | 当前代码范围 | 本轮证据 |
| --- | --- | --- |
| Codex 原生 Responses 图片输入 | 保留 input_image、URL／data URL、detail 和原生工具声明；复用原 OAuth | 最终出站 fixture；未向真实 Codex 发图片 |
| Chat 图片输入→Codex Responses | image_url 转 input_image；本轮补齐丢失的 detail | 出站 fixture；未验收模型图像理解 |
| Codex 图片生成／返回 | 不把普通文本成功算图片验收；原生工具声明可以保留，但没有证明账号允许生成或图片事件／返回格式完整互通 | 未完成真实及完整返回 fixture 验收；不宣称 Chat 图片结果互转或独立 `/images/*` 已兼容 |
| DashScope OpenAI 兼容 Chat | 原 gpt/OpenAI 兼容渠道配置 compatible-mode/v1，保留模型映射、stream、tools、tool_choice、usage 请求选项 | 最终 URL／header／body fixture；未实测流式工具参数及供应商 usage |
| DashScope 原生多模态／其他原生接口 | 不能与 OpenAI 兼容接口混为一谈；已有 ASR 处理不代表原生图片／多模态接口全覆盖 | 本轮未新增专用协议，不宣称全部支持或全部不支持；需按实际使用的接口与模型逐项验收 |

真实验收必须分别检查认证、带图片请求、图片输出／返回格式、SSE 终止、工具参数和 usage，不能以 HTTP 200 或普通文本结果替代。协议参考应使用供应商官方文档；当前尚无针对 ChatGPT 订阅后端图片能力的上游实测结论。

## 插件

上传 ZIP 即由 Miora 托管的流程、SDK、OAuth 扩展、配置迁移及版本清理见 [Python SDK](../sdk/python/README.md)。独立服务协议仍保留，见 [渠道插件](channel-plugins.md)。本轮不做在线自动检查、下载或执行插件更新。
