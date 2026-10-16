# 内置管理控制台

## 范围与运行方式

AI 供应商与模型使用 `assets/admin/ai-icons/` 中的 114 个本地品牌 SVG，来源与原始许可证保存在该目录的 `SOURCE.md` 和 `LICENSE`。渠道列表、供应商选择、插件适配器显示供应商品牌；模型候选和已选模型按上游 ID 显示模型家族，用量明细与请求日志按记录的模型名显示。无法识别的公开别名保留通用图标，不额外请求模型目录。OpenRouter 等平台的渠道图标与其承载的 Claude、Gemini 等模型图标分别匹配，未知兼容接口不一律标成 OpenAI。黑白标识随深浅主题调整，彩色标识保留原色，不加彩色底板；所有图标仅通过同源静态路由加载。

控制台使用 HTML、CSS、原生 JavaScript ES 模块，无 npm、前端构建步骤、外网 CDN 或图标字体依赖。导航、按钮和状态图标由本地 `icons.js` 输出 inline SVG；品牌统一使用 Git 仓库选定的 `assets/branding/miora-logo.jpg` 银色折带 M，不重新绘制为其他标识。`favicon.svg` 内嵌由原图裁切缩小的 JPEG，登录页、侧栏和浏览器标签共用该文件，不加载外部图片。用户界面为中文，包含桌面与窄屏布局。

视觉采用黑白灰层级和细边框，支持浅色、深色与跟随系统；公共容器与表单控件的圆角不超过 4px，圆形状态点与加载指示器除外。不使用装饰性渐变、多彩图标底板或卡片投影。概览指标共享一个网格面板，窄屏改为两列；成功、警告和危险状态保留语义颜色。焦点轮廓、侧栏抽屉、表格内部横向滚动和 reduced-motion 行为不变。

登录页及控制台右上角提供「外观」选择器，默认跟随系统并实时响应系统配色变化。`theme.js` 是先于样式表执行的同源小型脚本，在首次绘制前应用已保存的偏好，避免深色加载时闪白。只有显式浅色 / 深色选择写入 `localStorage` 的 `miora.theme`；跟随系统移除该项。不同标签页自动同步。主题切换不请求后台、不重绘页面数据、无轮询；禁止浏览器存储时仍可在当前页面切换。主题偏好不是账号配置，不保存任何凭据。

品牌资源用 `data-source` 记录原始 JPG 路径，不做哈希完整性校验。若更换原图，应同步重新裁切生成 `favicon.svg` 内嵌图片；不要只修改 README 或恢复独立的 `brand` 线条图标。当前裁切区域为原图 `(230, 235, 1030, 1035)`，缩为 384×384 JPEG（质量 92），保留银色渐变的细节，无需前端运行时依赖。

静态文件由 Rust `include_str!` 嵌入；**修改静态资源后必须重新构建 Rust 二进制**。直接打开 HTML 文件不能工作：控制台必须和后端同源运行。

渠道列表支持源配置级永久启停与批量启停/分组；「凭据」「虚拟模型」和侧栏「对话调试」分别管理账户、后备链与流式推理。配置示例和真实支持边界见 [网关入口、路由与调试](gateway-features.md)。

| 源文件 | 服务路径 / 用途 |
| --- | --- |
| `assets/admin/login.html` | `/login`，管理员密码登录 |
| `assets/admin/theme.js` | 登录页 / 控制台共享的首屏主题引导和外观选择器 |
| `assets/admin/index.html` | `/admin`，控制台外壳 |
| `assets/admin/styles.css` | `/admin/assets/styles.css`，布局、表单、响应式与可访问性样式 |
| `assets/admin/app.js` | `/admin/assets/app.js`，hash 路由与业务页面入口 |
| `assets/admin/plugin-manager.js` | `/admin/assets/plugin-manager.js`，原生插件文件上传、已安装列表与内置适配器分组 |
| `assets/admin/workspace.js` | `/admin/assets/workspace.js`，文件浏览、源配置编辑、只读文档及下载 |
| `assets/admin/backend-logs.js` | `/admin/assets/backend-logs.js`，运行事件增量读取、筛选、暂停与跟随 |
| `assets/admin/model-testing.js` | `/admin/assets/model-testing.js`，真实单模型 / 串行批量测试及请求日志下钻 |
| `assets/admin/playground.js` | `/admin/assets/playground.js`，Chat / Responses 三路流式多轮对比、思考折叠与取消 |
| `assets/admin/channel-accounts.js` | `/admin/assets/channel-accounts.js`，逐凭据启停、冷却/失败状态、导入及 Codex 刷新 |
| `assets/admin/virtual-models.js` | `/admin/assets/virtual-models.js`，有序后备目标、嵌套虚拟名及状态条件 |
| `assets/admin/key-manager.js` | `/admin/assets/key-manager.js`，凭据名称、分组、权限、速率、持久化启停及吊销 |
| `assets/admin/channel-editor.js` | `/admin/assets/channel-editor.js`，渠道三页草稿、保存与真实请求预览 |
| `assets/admin/parameters-editor.js` | `/admin/assets/parameters-editor.js`，能力驱动参数表单与无损策略编辑 |
| `assets/admin/request-logs.js` | `/admin/assets/request-logs.js`，真实请求日志、阶段内容与两步清理 |
| `assets/admin/log-settings.js` | `/admin/assets/log-settings.js`，持久日志与隐私配置、容量单位转换 |
| `assets/admin/login.js` | `/admin/assets/login.js`，登录交互 |
| `assets/admin/api.js` | `/admin/assets/api.js`，同源 API / CSRF 客户端 |
| `assets/admin/models-editor.js` | `/admin/assets/models-editor.js`，模型候选、筛选选择与逐项别名编辑 |
| `assets/admin/ui.js` | `/admin/assets/ui.js`，转义、schema 表单、对话框与通知 |
| `assets/admin/icons.js` | `/admin/assets/icons.js`，固定本地 SVG 图标模块 |
| `assets/admin/favicon.svg` | `/admin/assets/favicon.svg` |

静态路由需包含 ES 模块的相对导入，包括 `ui.js`、`models-editor.js`、`channel-editor.js` 与 `parameters-editor.js`。不使用内联 script、style 或事件属性，可配合 `script-src 'self'; style-src 'self'; connect-src 'self'` 的 CSP。

### 路由

- `/admin#overview`：概览
- `/admin#channels`：渠道与模型
- `/admin#plugins`：插件管理
- `/admin#interceptors`：请求拦截器
- `/admin#keys`：API Key
- `/admin#workspace`：工作区
- `/admin#testing`：模型测试，可由渠道或 API Key 行直接进入
- `/admin#backend-logs`：后台运行日志，内存保留最近 2000 条，重启清空
- `/admin#logs`：真实请求日志
- `/admin#usage`：用量与管理审计（仍可从日志页子导航 / 概览进入）
- `/admin#settings`：系统设置 / 请求调度
- `/admin#log-settings`：系统设置 / 日志与隐私

空 hash 默认概览；未知 hash 归一到概览。登录成功固定跳转 `/admin#overview`，不接受任意 return URL。

渠道列表的搜索词、接入方式与分组保留在当前页面 URL 参数中（`#channels?q=...&type=...&group=...`）；密钥列表保留分组、搜索和启停状态筛选。筛选在本地执行，不额外请求接口。渠道和密钥的分组参与真实路由权限，未指定时属于 `default`。编辑弹窗支持 `Ctrl+Enter` / `Command+Enter` 提交，不自动重试写入。

工作区的源配置包含完整凭据，打开前明确确认；其他文件只读，不开放任意磁盘路径、环境文件或数据库。源配置保存保留原始 YAML/JSON 文本，包含注释，按修订号处理并发冲突。远程配置只读、脱敏。模型测试需要手动运行，可能产生真实费用；不设置默认输出 Token 预算，批量按顺序执行。精确渠道测试保留现有管理员 API Key + Responses 的权限要求；目标不可用时禁用测试，不暗中回落到其他渠道。

## 认证与安全边界

1. `POST /admin/api/login`，请求 `{username,password}`。
2. `GET /admin/api/session` 返回 `{user:{username,role},csrf_token,expires_at}`，前端仅在模块内存保存 CSRF。
3. 会话使用服务端 HttpOnly / SameSite=Strict Cookie。fetch 固定 `credentials: 'same-origin'`、`cache: 'no-store'`。不将密码、会话、CSRF 或 API Key 写入 localStorage / sessionStorage。
4. 除 GET / HEAD 外的写操作添加 `X-CSRF-Token`。`POST /admin/api/logout` 撤销会话，成功后跳 `/login`。
5. 会话接口或业务接口 401 跳登录。登录失败不自动跳转，避免登录循环；503、429、403、409 等显示后端真实错误。
6. 错误契约为 `{error:{code,message}}`。409 明确要求刷新，不自动重试写入；普通管理请求 25 秒超时，手动模型实测使用 180 秒服务端时限与 185 秒客户端时限。路由变更中止过期加载，避免旧页面覆盖新页面。
7. 后端文本必须通过 `esc()` 或 `textContent` 进入页面；后端不允许提供 HTML / SVG。动态图标名称仅查找固定本地图标字典。
8. 模态框关闭时清空输入值并移除表单 DOM；一次性新 Key 不留在隐藏对话框中。剪贴板仅在用户点击复制后使用。

生产应使用 HTTPS；Secure Cookie 默认开启。仅在本地 HTTP 测试时设置 `MIORA_ADMIN_SECURE_COOKIE=false`，并使用匹配的 `MIORA_ADMIN_ORIGIN`。后台启用和密码策略详见后端认证文档；前端不会创建默认管理密码，也不会拿模型 API Key 代替管理登录。

## 已对齐的管理 API 契约

所有路径以下均省略公共前缀 `/admin/api`。后端为认证 / 管理模块以及插件模块；前端不猜测替代路径。

### 概览

`GET /overview`：

```text
{runtime, version, configuration_ready, database_disabled,
 api_key_count, provider_count, model_count, stats, runtime_details, capabilities}
```

概览同时读取 `/overview` 的配置清单与 `/usage` 的持久化模型统计。渠道数、配置模型数和 API Key 数仅为紧凑清单，不作为使用量。用量面板展示请求与成功 / 失败数、输入、输出、缓存命中 / 未命中 / 写入、命中率、USD 费用估算及平均耗时 / 首字耗时；模型明细可查看总 Token、1 小时缓存写入及字段报告覆盖率，并点击模型进入请求日志。支持最近 1 / 7 / 30 / 90 / 365 天以及模型、渠道精确筛选。数据库禁用或字段未报告时显示 `—` / 空态，真实 0 保留，不填模拟趋势或健康指标。

### 渠道与模型

- `GET /channels` → `{revision,channels:[{provider,base_url,engine,channel_plugin,models,api_key_count,temporary}],writable}`。
- `GET /channels/detail?provider=<encoded name>` → `{provider,revision,base,effective,schema,...}`。**编辑使用详情响应的 revision**；详情为脱敏源配置，不是运行时覆盖视图。
- `POST /channels` → `{revision,channel:{provider,base_url,engine,api,model,...}}`。
- `PATCH /channels` → `{revision,provider,patch:{...}}`。
- `DELETE /channels` → `{revision,provider}`。
- 成功返回 `{ok:true,revision,persistence:"source_file"}`。

仅 `writable:true` 显示真实写入入口。配置源为远程 URL 或临时渠道时只显示详情。创建必须有明确非空 `model` 数组，不能使用 `all` 触发隐式发现。模型选择与别名使用可视化表单，不再要求填写模型 textarea 或模型 JSON。

#### 渠道类型与大面板

- 渠道编辑为约 1000px 的宽对话框，上方固定「基础 / 模型 / 参数与请求修饰」三页标签；基础页连接信息双栏，模型流程保持原样。手机使用全屏编辑、固定 tabs 与底部保存区，内容独立滚动，切页不丢草稿。
- 仅一个「渠道类型 / 供应商」选择器。选项来自 `/plugins` catalog；默认推荐 OpenAI 兼容、Anthropic Claude、Google Gemini、Codex、OpenRouter、Azure OpenAI 六类，服务插件动态追加。
- 旧配置中的其他引擎若未出现在推荐目录，仍保留当前类型并可编辑，不删除渠道、不悄悄转换类型。
- `manifest.default_base_url` 用作新建时的默认地址；手动修改过的地址不再随供应商切换被覆盖。
- 已有内置渠道允许改选另一个内置供应商，需确认对应 URL 和凭据；已有内置与服务插件之间不直接转换。
- 浅色侧栏、黑白灰层级与细边框；桌面正文 / 按钮 / 表格以 13–14px 为主，手机输入控件使用 16px。插件卡片采用桌面三列 / 中屏两列 / 窄屏一列。
- `styles.css` 按组件分区，响应式规则集中在文件末尾并由宽到窄排序。长页面由 shell 延续侧栏背景，渠道编辑与日志详情的标题、标签及操作区和滚动内容分离。

#### 获取、选择与别名

`POST /channels/models`：

```text
请求：{provider?: 已保存渠道名称,
       channel:{engine?,base_url?,api?,channel_plugin?,plugin_config?}}
响应：{models:[{id,name?}],source:"upstream"|"plugin"|"catalog"}
```

该请求带正常 session 与 CSRF，无需 revision，只读取目录，不保存配置、不触发模型生成。已保存渠道通过 provider 在服务端合并真实配置；未修改的 `base_url` 与空白 API Key / secret 不发送，沿用已保存值。Codex 可返回内置目录，Azure 不支持标准发现时显示后端说明并允许手动填写部署名。

- 「获取模型」成功只合并候选项，不自动全选、不清空已有选择或别名；失败显示后端错误，可继续手动添加。
- 可按 ID / 名称搜索；「全选筛选结果」「取消筛选结果」只作用当前可见候选，不影响过滤外的模型。
- 手动输入按 Enter 添加；粘贴支持换行、逗号、空格及中文分隔符。
- 已选模型逐项行内编辑上游 ID 与对外别名，可移除单项，或为同一上游添加另一个别名；修改上游 ID 后只刷新候选勾选状态，不重建正在输入的行。
- 已选列表可独立按上游 ID 或别名筛选，行操作始终对应原始模型项，不会因筛选改变索引。手动添加模型或新增别名时清除已选筛选，便于继续编辑新增项。
- 「批量设置价格」只作用当前已选列表中可见的模型项，直接更新每个模型的价格，不修改渠道默认价格。空白保持原值，明确填写 0 表示免费；未填写项、筛选外模型及历史费用不受影响，保存渠道后生效。批量输入在点击应用时统一校验一次，不逐模型重复校验；仅搜索或填写尚未应用的批量值不使请求预览过期。
- 模型搜索框按 Enter 不提交渠道；批量价格输入按 Enter 只应用到当前草稿。保存渠道使用原有按钮或 `Ctrl+Enter` / `Command+Enter`。
- `['id', {'upstream':'public-a'}, {'upstream':'public-b'}]` 中两个映射都保留，不能用 upstream 作为唯一行键。保存仍输出后端原有字符串 / 映射数组格式。
- 勾选已拥有别名的模型不会覆盖别名；取消包含别名的候选需明确确认。切换供应商只重置候选目录，已有选择与别名保留。
- 对外名称重复会明确报错，避免后端按名称覆盖。关闭或取消对话框不保存，进行中的目录读取会中止。

高级 JSON 仅用于其他渠道参数，不能覆盖主表单的供应商、地址、凭据或模型字段。

凭据输入不回显；空白不传，保留已有密钥。已有渠道的上游地址未修改时不回传 `base_url`，避免用脱敏显示值覆盖源 URL 的敏感查询参数。服务插件渠道设置 `channel_plugin` 与 `plugin_config`，无需填写内置 `engine/base_url`；配置表单由插件 `manifest.config_schema` 驱动。内部 `plugin_config` PATCH 为合并，未修改的 secret 一律省略，不把 `***` / `********` 当新凭据提交。现有渠道不能在 UI 直接切换内置/插件类型；需要新建目标渠道后删除旧渠道，避免服务端双来源配置残留。

### 渠道参数与真实请求预览

这部分独立实现在 `parameters-editor.js` 和 `channel-editor.js`，不是向 `app.js` 继续堆叠表单。仅按公开产品文档的行为需求独立实现，不引入外部前端组件。没有尚未实现的 OAuth、调度、流水线或发送测试按钮。

策略保存为 `preferences.parameter_policy`：

```json
{
  "defaults": {},
  "channel": {"set": {}, "remove": []},
  "models": [{"match": {"requested_model": "public-alias"}, "set": {}, "remove": []}],
  "headers": [{"name": "X-Tag", "mode": "set", "value": "", "enabled": true, "secret": false}],
  "system_prompt": {"mode": "inherit", "value": ""}
}
```

普通生成参数表单编辑**渠道层**；默认层及按公开模型覆盖可在完整策略 / 模型规则 JSON 中编辑。每次普通字段修改都克隆原策略，仅精确 patch 对应路径，保留未知根字段、嵌套扩展、其他 scope 与 preferences。高级 JSON 是完整 policy，不是只拼已知字段的导出副本；敏感头的 value 在高级展示中省略。

- 继承：移除渠道 `set` 中该路径且移除同路径的 `remove`，让请求 / 默认层继续生效。
- 设置：写入能力声明的 wire JSON Pointer 对应嵌套对象。`0`、`false`、`""` 和 `null` 均按值保留，是否允许该值由后端协议约束决定。
- 删除：从 `set` 移除字段，单独加入 `remove` 数组，不将 null 当删除哨兵。
- 输出预算仅写当前能力给出的一个字段。用户明确编辑预算时清除 `channel.set` / `channel.remove` 的其他已知预算别名，并识别旧别名的现有设置或删除状态；不修改 defaults、模型规则或未知扩展。
- 请求头支持添加 / 移除规则、继承 / 设置 / 删除、enabled 与 secret。已保存敏感值不回显；未编辑时不发送 value。用户点击「显式清空值」发送 `value:""`，与继承或省略不同。Authorization、Cookie、API Key / Token 类头即使未标 secret 也按敏感处理。名称不区分大小写且必须唯一，disabled 行也不能重名；旧策略有重复名称且秘密来源不明确时，需显式重新填写保留行值，不能默取第一行。
- 系统提示默认继承；前置、追加、替换、移除均需明确选择，不自动覆盖用户原系统消息。
- 未支持的 effort、thinking、service_tier 不显示为可用表单选项。

两条新接口均为 session + CSRF POST：

```text
POST /channels/parameter-schema
POST /channels/preview
{
  provider?: 已保存渠道名称,
  channel: {本次显式草稿字段，含 preferences.parameter_policy},
  request?: {endpoint, model:公开模型名, stream, body:示例请求体}
}
```

schema 返回 `{capabilities,revision,saved}`。capabilities 包含当前 `endpoint`、真实支持的 `endpoints`、`profile`、`scope`、`body_supported`、`header_supported`、`generation` 映射、`system_prompt`。生成字段仅采用 supported=true 的 temperature / top_p / output_limit / stop；`path` 是 RFC6901 JSON Pointer，例如 Claude `/max_tokens`、Responses `/max_output_tokens`、Gemini `/generationConfig/maxOutputTokens`。`native_preview` 只表示原生 Responses 专用构造器，**不是所有预览的总开关**；普通 Chat / Claude / Gemini 不因此被禁用。

preview 返回 `{scope,method,url,headers,body,requested_model,upstream_model,policy_trace,policy_report,capabilities,revision,saved}`。前端展示后端构造的接口、脱敏头 / 请求体、模型映射、`path/source/action` 来源表和约束警告，不自己拼最终 URL。示例请求不预置任意小 Token 预算。预览只构造、不发送、不保存；不要求未保存草稿有渠道名称。后端 422 `preview_unsupported` 如实显示，不能假装成功。

服务插件 `scope=plugin_envelope` 时明确显示「宿主执行信封，不是插件内部最终请求」；不据此推断插件内部供应商 URL 或认证。`header_supported=false` 时请求头表单不可操作，不能改控制面 Authorization。

已有 provider 的密钥与未修改 URL 都在服务端复用：不回传遮罩值，即使切换引擎，未修改的显示 URL 也不被当成新地址。保存仍使用 `/channels` POST / PATCH 和详情 revision 的 CAS，热生效且不重启；409 要求重新加载，不自动覆盖别人配置。

后端合并约定：普通 preferences 深合并；显式 `parameter_policy` 是完整的已知策略语义替换，使 `set:{}` / 删除已知段真正恢复继承；未知扩展保留。显式 headers 数组替换整表，删行生效；同名敏感头 value 省略保留已有秘密、空串显式清空。完全不传 parameter_policy 才表示保留原策略。

### 插件上传与版本管理

`GET /plugins`：

```text
{plugins:[{id,kind:"builtin"|"service",enabled,revision,manifest,endpoint,
 config,service_auth,versions:[{revision,version,active,leases,draining}]}],
 trust:{allowed_origins,allow_loopback_http}}
```

安装与更新只需选择或拖入原生 `.miora.json` 文件，预览名称、ID 和版本后提交。**没有默认配置、上游地址、API Key 或服务 Token 输入框**。已安装服务与内置适配器分组展示；上游认证及参数在渠道编辑器填写。插件声明中的字段结构供渠道使用，不是安装向导。

文件内容采用 Miora 自有 `miora.plugin.v1` 格式，包含 `format`、`manifest`、`endpoint`，以及可选的 `config` 和 `service_auth`。它描述已有独立运行的 Miora 插件服务，不是 Python/ZIP 源码包，不启动进程，也不兼容其他项目的插件协议。可从 `GET /plugins/template` 下载原生模板。文件中的 `manifest` 支持：

```text
config_schema:[{key,label,type,required,default,enum,secret}]
type: string | integer | number | boolean | object | array
auth:{type:"none"|"bearer"|"api_key",config_key,header}
capabilities:{endpoints:[...],streaming,model_discovery}
models:[{id,name}]
```

- `POST /plugins/upload` 使用 `multipart/form-data`：一个 `file` 字段，以及可选的 `options` JSON `{expected_revision}`。首次修订为 0，更新使用当前修订；不接受配置/认证覆盖。文件上限 256 KiB，请求体上限 512 KiB，拒绝重复文件和未知字段。服务健康检查成功后原子发布，409 不自动重试。
- `POST /plugins` 保留原有 JSON 注册接口供工具使用：`{manifest,endpoint,config,service_auth,expected_revision}`；管理页面不再提供手工注册表单。
- `POST /plugins/{id}/enable` / `disable`：`{expected_revision}`。
- `POST /plugins/{id}/rollback`：`{revision:目标执行版本,expected_revision:当前配置修订}`。
- `POST /plugins/{id}/models`：`{config:{临时覆盖}}` → `{models:[{id,name}]}`。发现模型会真实调用服务；空白 secret 不发送，使用服务端默认配置。发现结果不自动写入渠道。

回滚切换 active 执行版本，并增加配置修订；**不声称创建新的执行版本条目**。版本表中 `active / leases / draining` 均为后端真实状态。停用、回滚需确认。内置插件仅展示详情，不能伪装成可热更新服务。服务插件无删除入口，通过停用保留历史。

信任策略只读，上传文件不能给任意地址授权。端点拒绝 / 认证 / schema 错误原样展示。HTTPS 服务需要文件自带独立控制面 Bearer，只有显式批准的本机 HTTP 才能无认证。上游凭据仍属于渠道，绝不通过上传表单补填。

### 请求拦截器

- `GET /interceptors` → `{revision,interceptors:[{id,enabled,order,plugin_ids,set_headers,remove_headers}]}`。
- `PUT /interceptors` → `{expected_revision,interceptors:[完整列表]}`。

独立于渠道插件，按 `(order,id)` 执行；空 plugin_ids 代表全部。提供实际新增、编辑、启停、删除操作；更新使用全列表 CAS，409 要求刷新。设置请求头使用 `name: value` 文本表单，不是 JSON dump；后端仍是允许头部范围的最终校验边界。只支持非敏感自定义 x-*，不能更改认证、Cookie、Host 等。

### API Key

后端本轮增加了真实最小 CRUD。UI 根据能力决定，不将早期只读契约硬编码成假按钮：

- `GET /api-keys` → `{revision,api_keys:[{id,position,role,models,masked_key}],writable,read_only}`。
- `POST /api-keys` → 请求 `{revision,models,role}`；返回 `{ok:true,revision,id,api_key,show_once:true}`。
- `PATCH /api-keys` → `{revision,id,models,role}`。
- `DELETE /api-keys` → `{revision,id}`。

默认 role=user，模型授权默认 `all`（不是 `*`）；要求非空授权数组。赋予 admin 权限必须额外勾选确认。完整 Key 仅在创建结果对话框显示一次；关闭后 DOM 清空，不缓存或写入存储。吊销必须危险确认。`writable:false` 时完全不显示创建、编辑、吊销按钮。

### 请求日志、用量与管理审计

三个概念明确分开：`#logs` 是新请求日志；`#usage` 保留用量汇总和管理员审计；旧 `GET /admin/api/logs` 的审计 DTO 没有改名或冒充请求日志。

- `GET /usage?days=1..365&model=<可选>&channel=<可选>` → `{available,summary,usage:[模型统计],from,to,currency:"USD",cost_kind:"configured_price_estimate",truncated,history_note:"reported_fields_only"}`。`summary` 与每个模型行均含 `request_count / success_count / failure_count`、`input_tokens / output_tokens / total_tokens`、`cache_read_tokens / cache_miss_tokens / cache_write_tokens / cache_write_1h_tokens`、各字段的 `*_samples`、`cache_hit_rate`、`estimated_cost / priced_requests`、`average_duration_ms / average_first_text_ms`。最多展示 500 个模型，总计仍包含完整筛选范围；兼容字段 `api_key_prefix` 固定为 `********`，不按密钥分组。

输入统一包含缓存读取和写入；原始 Anthropic 普通输入需与缓存相加，OpenAI / Gemini 已包含缓存，不重复累加。未命中只对同时报告输入与读取且读取不大于输入的请求计算，命中率也使用同一批有效配对数据。总 Token 仅累加同时报告输入与输出的请求，不把不同覆盖范围的汇总直接相加。旧记录只保留历史 prompt / completion 计数，没有的缓存和价格事实不补零。

「渠道与模型 → 添加 / 编辑 → 基础 → 协议处理」提供兼容适配（`preferences.protocol_mode=auto`，保留现有行为）与原生透传（`native`，不做 Chat / Responses 协议转换）。通用开关仅支持 gpt / openrouter / codex，Codex 原生模式仅接受 Responses / compact；其他供应商和服务插件使用各自适配器 / 协议声明。透传不是逐字节转发：认证、模型映射、参数规则以及 JSON 重编码仍生效。

价格入口位于渠道编辑器的「模型」页，每个已选模型都有独立的「价格」展开区，可配置普通输入、输出、缓存读取、5 分钟缓存写入、1 小时缓存写入的 USD / 百万 Token 单价（`preferences.usage_pricing.models[公开模型名]`）。0 表示免费，留空不计价；已有渠道级默认价格会载入各模型，修改不会串到其他模型或渠道。改别名时价格随公开模型名迁移，删除已选模型同时清理对应价格，不删除未载入的历史目录项。`usage_pricing.models` 在管理 API 中按完整目录替换，未提交的其他渠道字段仍按原有规则合并。每次请求只存储实际渠道和具体模型的价格快照，后续改价不重算历史。输入 / 输出、有效缓存数据或所需单价缺失时不产生估算；展示已计价请求覆盖数。此金额不是供应商账单，不含未观测到的失败 / 重试 / 其他供应商收费。统计独立于原始请求日志，清理日志不删除用量。
- `GET /logs` → `{scope:"admin_audit",logs:[{timestamp,action,outcome}],persistent:false}`，仍是当前进程内管理审计。
- `GET /request-logs?page=1&page_size=50&request_id=&channel=&model=&status=&from_ms=&to_ms=` → `{rows,total,page,page_size}`。

新日志列表只读摘要，不加载大正文。筛选为实际支持的 Request ID / 渠道 / 模型 / HTTP 整数状态精确匹配，以及 Unix 毫秒 `[from_ms,to_ms)` 时间范围；界面时间为本地时区。模型筛选匹配 requested / sent / response 任一实际模型名。错误阶段可显示，但本轮没有错误阶段 / Key ID 筛选 API，因此不放伪筛选项。

行字段使用 `at_ms`、`request_id`、`channel`、`requested_model`、`status`、`duration_ms`、`first_byte_ms`、`first_text_ms`、`usage`、`attempts`。未观察指标显示 `—`，真实 0 保持 0。attempts 包含 hedge / repair 等派发，不能用长度减一伪造重试数；列表明确分开未提供的重试数与实际尝试数。

#### 阶段详情与安全内容

`GET /request-logs/{id}` 返回 `{record,fields,active}`，字段元数据包含：

```text
{field,attempt:null|实际序号,observed_bytes,saved_bytes,complete,
 capture_state,availability,available,reason,availability_reason,
 content_type,representation,redacted}
```

抽屉页签为概览 / 客户端请求 / 上游请求 / 上游响应 / 返回客户端 / 尝试路径。上游字段按返回的真实 attempt 序号选择，不假定数组位置或 0/1 起点，不用最终尝试覆盖所有 retry 的内容。

- `observed_bytes` 是对应 representation 中真实观察到的实体字节，`saved_bytes` 为实际保存的脱敏字节。客户端实体可能是解压后的内容，不等同于 TLS / 压缩网络字节。
- 字段 `complete` 表示捕获时整份字段已安全保留，截断 / 丢弃为 false；界面称「字段捕获完整性」，不将它等同于当前文件可用性。
- `upstream_complete` 和 `relay_complete` 独立显示；后者仅指转交 HTTP 栈结束，不能宣称客户端已确认收齐。未观察到的 `response_model` 必须为 `—`。
- `capture_state` 与持久 `availability` 分开显示。disabled、retention_zero、unsupported、truncated、redaction_failed、size_limit、memory_limit、not_observed，以及 expired / cleared（已清理，兼容旧 cleaned）/ evicted 均保留实际原因。

内容只在点击后读取：

```text
GET /request-logs/{id}/content?field=response_body&attempt=<可选>&limit=65536
GET /request-logs/{id}/download?field=response_body&attempt=<可选>
```

content 返回安全文本 JSON 包装：`content:string,encoding,preview_truncated,field,attempt,saved_bytes,observed_bytes,capture_state,availability,available,complete,reason,content_type`。正文始终使用 `pre.textContent`，可选择 JSON 格式化视图；HTML / SVG 不经 innerHTML 执行。列表与初始详情都不预取正文。

仅在 `available===true && availability==="available"` 时提供内容与文件按钮。下载前再次读取详情确认当前可用性，下载使用固定同源路径和后端 octet-stream 附件，而不是任意后端 URL。无文件时由后端 404 / 410 拒绝，不制造客户端“完整文件”。

`preview_truncated` 表示文本预览长度有限；`capture_state=truncated` 表示存储内容本身只有部分，两者有不同提示。按钮叫「下载已保存文件」，不把捕获前缀称为完整原始传输。清理 / 过期 / 淘汰后刷新会移除查看与下载操作。

#### 两步手工清理

1. 选择 `clear_fields`（八个原始字段）或 `delete_rows`（新请求日志行），填写时间 / Request ID / 渠道 / 模型 / HTTP 状态筛选。
2. `POST /request-logs/cleanup` 请求 `{dry_run:true,action,fields,filters}`，返回冻结目标的 `confirmation_token,expires_at_ms,matched_rows,active_excluded,estimated_bytes,action,fields,filters`。
3. 确认窗口展示实际匹配数、排除活跃行、预计字节和可读筛选范围，再提交 `{dry_run:false,confirmation_token}`。token 有效期 5 分钟，过期需重新预览；执行期间的新请求不加入本次目标。
4. 执行结果固定面板展示 `affected_rows,cleared_fields,freed_bytes,skipped_rows`，部分跳过会明确提示，不只弹一个成功 toast。

`clear_fields` 保留日志摘要；`delete_rows` 只删除新请求日志，不删除原 `stats` / 用量摘要。空筛选意味着全部符合条件的日志，不仅当前分页，会在确认中明确提示。进行中的请求由后端排除。

### 日志与隐私设置

`#settings` 顶部直接提供「请求调度 / 日志与隐私」入口；`#log-settings` 是独立表单，不藏在高级 JSON。四区包含启用与八字段捕获、原始内容保留小时、日志行策略、容量及条件可管理的运行日志轮转。

```text
GET /log-settings -> {revision,settings,backend_enabled,runtime_rotation_managed,...}
PUT /log-settings <- {revision,settings}
settings = {
 enabled,capture_fields:[...],body_retention_hours,
 row_retention:{mode:"manual"|"days"|"forever",days},
 limits:{per_request_bytes,total_body_bytes},
 runtime?:{file_max_bytes,retained_files}
}
```

八字段为 request_headers / request_body / upstream_request_headers / upstream_request_body / upstream_response_headers / upstream_response_body / response_headers / response_body。默认 capture none；`body_retention_hours=0` 不保存原始头和体，不表示永久保存。已禁用、0 小时、总预算 0、未选择字段分别说明原因。

界面使用 KiB / MiB，分别乘 1024 / 1048576 精确保存 bytes，不将空输入当 0。单请求上限为 1 字节至 16 MiB（0 报错）；总内容预算允许 0，表示没有原始内容容量、会淘汰内容但保留摘要。manual / forever 保留已有 days 数字，只有 days 模式校验 1–36500 天。

只有 `runtime_rotation_managed=true` 才出现可编辑轮转字段：单文件 1 KiB–1 GiB，1–32 个文件（包含当前文件）。其他部署显示「由部署环境管理」，PUT 不发送 runtime，不放无效开关。`backend_enabled=false` 明确展示后端不可用并禁止保存，不能把 DISABLE_DATABASE stub 伪装成持久配置。若 `status.initialization_error` 返回目录 / SQLite 初始化失败原因，则直接安全显示真实原因；null 的禁用 stub 不伪报初始化故障。

配置 CAS 持久化、热生效，无需重启。409 保留本地表单草稿并要求用户主动重新载入；成功后读取服务端配置再确认保存结果。

### 系统设置

- `GET /settings` → 原生 `{provider,revision,effective,resolved_fields,schema,...}`，默认 provider 为 `__uni_console_global_preferences__`。
- `POST /settings/validate` 与 `PATCH /settings` 使用 `{revision,operation_id,changes:[{provider,set:{JSON_POINTER:value},remove:[],reset:false}]}`。

基础表单支持 model_timeout、keepalive_interval、四类 cooldown、hedging；高级区域支持 timeout_policy。优先使用 `resolved_fields[path].value`，不将缺失字段伪造为服务端值。只提交变更；保存只发送一次 PATCH，由后端验证、原子写入源配置并热生效，重启后保留，不依赖数据库。「仅校验」独立发送 POST，不写入文件或更改运行时。远程 CONFIG_URL 明确显示只读并禁用表单，不能伪装为保存成功。

## 可访问性与交互

- 原生表单 label / required / 类型校验，错误 role=alert；成功消息 aria-live。
- 请求加载、空数据、网络错误、409 冲突、只读能力分开呈现。
- 原生 `<dialog>` 提供焦点限制与 Escape；写入进行中阻止重复提交及意外关闭。
- 删除渠道、吊销 Key、启停插件、回滚及删除拦截器需确认。
- 所有图标为装饰性 aria-hidden，纯图标按钮都有中文 aria-label。
- 窄屏侧栏关闭时 inert，不会留下不可见的键盘焦点目标；展开支持 Tab 边界循环及 Escape 返回菜单按钮。
- 支持系统 reduced-motion，表格可独立横向滚动，不使窄屏整页横向溢出。

## 自动检查与 QA

### 小范围格式化复用与冗余清理

本轮仅复用 `ui.js` 的固定 `zh-CN` NumberFormat、移除会话失败页重复刷新监听，以及两个未用导入和渠道编辑器只写的 `activeTab`；未改导出接口、CSRF、保密规则、界面或框架结构。

```powershell
node --test --test-name-pattern="admin cleanup:|unknown metric" tests/ui/admin_modules.test.mjs
```

五个改动模块语法通过，定向测试 **4/4**。同一 Node **v22.19.0** 下，对 250 个相同有限数值逐项比对旧表达式与真实 `ui.js` 的 `fmt`：输出一致，NumberFormat 构造次数由 **250 次变为 1 次**。这是构造次数验证，不是耗时或整体页面加速倍数。

测试还执行实际 `app.js` 的会话错误路径，保留错误信息与登录入口；指针 click 和键盘激活产生的 click 均只调用一次 reload。该项使用 Node DOM 桩，未声称验证浏览器按键合成。真实页面点击 / 键盘冒烟由父任务统一新二进制后进行；本轮未自行 build、启动浏览器或触碰 8000。

### 日志阶段必要检查

```powershell
Set-Location G:\miora
node --test --test-name-pattern="log UI:" tests/ui/admin_modules.test.mjs
```

本轮仅新增 11 项纯函数用例：八个准确捕获字段、单位转换 / 0 与空值、实际数值约束、managed / unmanaged 轮转、禁用与零保留解释、精确分页筛选、真实 attempt 内容路径、可用性与截断分离、两步清理意图、捕获完整性及未知指标。全部通过；四个改动 JS 模块通过 ES 模块语法检查。

2026-10-14 已使用父任务 P120 最终构建的二进制完成真实 Chrome 验收（P122，退出码 0）。先把 exe 复制到 `.artifacts` 下的临时目录，不占用 target，不重启 / 修改实际 preview；实例使用独立 SQLite、请求日志目录和随机管理员凭据，上游仅为本地 mock。

通过范围：

- “日志与隐私”可见入口、八字段默认 none / 0 小时、KiB / MiB 精确字节、非托管轮转无无效控件；保存 / 刷新回显一致，并直接确认 SQLite 已持久保存设置。
- 真实模型请求产生列表；四个捕获阶段按需读取，attempt 使用真实序号；请求头秘密未回显。
- SVG / HTML 样式攻击字符串以文本展示，未创建 SVG 元素或执行 onload；JSON 格式化不改变存储内容。大内容返回 preview_truncated，附件字节数等于真实 saved_bytes，而不是伪造“全文”。
- dry-run 后新增同条件请求，确认只清理冻结目标；清理状态 canonical 为 `cleared`，显示“已清理”并撤下下载。已清内容返回 404 / 410；`delete_rows` 删除新日志行后，旧用量 / stats 请求数不变。
- 0 小时产生摘要但无原始内容，界面说明原因；390px 设置与详情抽屉无横向溢出，原用量 / 审计路由仍可达；零 JavaScript 异常。
- 日志目录故意不可初始化时展示真实原因并禁用写入，模型推理仍成功；disabled stub 不误报为初始化失败，也不能伪保存。

首轮发现协议文字曾写 cleaned，而后端实际为 cleared；已补 canonical 中文映射并在 P120 嵌入资源上完整复验。测试框架 SQLite 连接改为显式关闭，最终临时目录正常回收。最终截图先 blur 焦点，未修改正常键盘 skip-link 功能。

产物位于 `.artifacts/admin-ui/`：`request-logs-browser-report.json`、`request-logs-browser-output.txt`，以及 `request-logs-desktop.png`、`request-log-detail-desktop.png`、`request-log-cleanup-result.png`、`log-settings-mobile.png`、`request-log-detail-mobile.png`、`log-settings-initialization-error.png`。已打开截图视觉检查。未使用实际预览数据或付费供应商，未执行旧全套、哈希或文件完整性检查。

### 参数编辑阶段必要检查（历史）

```powershell
Set-Location G:\miora
node --test --test-name-pattern="parameter editor:" tests/ui/admin_modules.test.mjs
```

本阶段新增 16 项策略 / 预览纯函数用例并全部通过：未知嵌套字段保留、继承与删除分离、0 / false / 空字符串 / null、默认层值、单预算字段、Gemini 嵌套路径、敏感头省略和显式空、认证头强制隐藏、完整 JSON 与按模型顺序、敏感草稿恢复、危险 Pointer 拒绝、generic 预览能力、未修改 URL 省略。五个改动模块通过 Node ES 模块语法检查。

本阶段已用父任务统一构建的临时 Rust 网关与本地 fixture 完成限定 Chrome 联调，不运行旧全套 UI 回归或资源比对：

- 三页切换保留基础、模型多别名与参数草稿；参数页直接保存而名称缺失时自动切回基础并聚焦必填项。
- 实际 preview 体现温度 0、单一输出预算、stop=null、top_p 删除、按公开别名覆盖以及系统提示追加；不产生上游网络调用，也不保存草稿。
- 完整 JSON 的 0 / false / 空字符串 / null 与未知根 / 嵌套扩展、其他 preferences 保存后仍存在。
- 新敏感头正确保存；已保存 secret 与未标 secret 的 Authorization 均不回显，后续请求省略其 value；显式清空发空字符串并清除旧值。
- 保存并重开回显一致；两个编辑窗口使用同一 revision 时，第二个收到 409、保留草稿且明确要求刷新，没有自动覆盖。
- 390px 编辑器全屏，标签与底部保存区不随内容滚动；无横向溢出，零 JavaScript 异常。
- 服务插件子场景使用正确 `/health` 握手注册，随后 preview 新增网络调用为 0；显示 `plugin_envelope` 和“宿主执行信封，不是插件内部最终请求”，控制面请求头操作禁用。首次 fixture 缺健康协议导致注册失败，修正 fixture 后单独重测通过，未为此修改产品逻辑。

已视觉检查 `.artifacts/admin-ui/channel-parameters-desktop.png`、`channel-parameters-mobile.png`、`channel-service-preview.png`。发现无上下界被序列化为 null 时提示不应显示“最大 null”，已仅将约束提示过滤为有限数值，参数本身的 null 值语义不变。该最后显示修正需随父任务最终构建嵌入。

### 模型编辑阶段必要检查（历史）

```powershell
Set-Location G:\miora
node --test --test-name-pattern="model editor:" tests/ui/admin_modules.test.mjs
```

本次在已有模块测试中新增 9 项模型纯函数用例：旧模型结构和同上游多别名往返、别名方向、保留原有映射的选择、仅对指定筛选集取消、目录候选合并、批量粘贴分隔符、非法旧结构明确拒绝、模型 ID 校验。9/9 通过；`app.js`、`ui.js`、`models-editor.js` 均通过 Node ES 模块语法检查。

不新增静态完整性检查或资源哈希，不运行上一轮全套 UI 回归。二进制由主任务统一构建 / 部署，前端不自行重启现有预览。

### 模型编辑阶段针对性浏览器验证（历史）

使用一次性临时真实 Rust 网关与仅实现 GET 模型目录的本地上游 fixture，测试目录获取 → 搜索与勾选 → 手动 Enter 添加 → 同上游多别名 → 保存并重开回显，以及 390px 面板和截图。不访问已有预览渠道的真实供应商、不使用真实供应商凭据，也不发送生成请求。

2026-10-13 已使用父任务统一构建的 `target/x86_64-pc-windows-gnu/debug/uni-api-front.exe` 完成本轮定向浏览器验证，全部通过：

- 六类单一类型选择器，无旧两级下拉；成功获取候选时已选数量仍为 0。
- 搜索后全选 / 取消仅影响当前筛选结果，未破坏过滤外的选择与别名。
- 手动 Enter 与批量输入、上游 ID 行内修改、Tab 后焦点连续。
- 同一个上游 ID 的两个不同对外别名保存后重开仍完整保留，源配置数组与预期一致。
- 编辑已保存渠道获取模型时未发送空 API Key 或未改的脱敏 URL；真实上游 fixture 收到服务端复用的凭据。
- 取消包含别名的候选可拒绝确认且保留原映射；关闭表单不保存未提交修改。
- 1440px 双栏和 390px 单栏，无整页或对话框横向溢出，零 JavaScript 异常。
- 已查看截图：`.artifacts/admin-ui/models-panel-desktop.png`、`.artifacts/admin-ui/models-panel-mobile.png`。模型列表可纵向滚动，窄屏的上游 ID / 别名均可编辑。

本次未运行旧全套 UI 回归或资源比对；上一轮认证 / CRUD / CSP 的完整测试结果属于历史验证。

### 现代化样式与布局回归（2026-10-15）

```powershell
python tests/ui/check_admin_assets.py --node D:/node.exe
D:/node.exe --experimental-default-type=module --test tests/ui/admin_modules.test.mjs
cargo build
# 使用已有项目内 Python 依赖；也可使用自行安装了 Playwright 的环境。
$env:PYTHONPATH = 'G:\miora\.tools\python'
python tests/ui/verify_admin_browser.py --binary target/debug/uni-api-front.exe --artifacts .artifacts/admin-ui-modern
python tests/ui/verify_request_log_browser.py --binary target/debug/uni-api-front.exe
```

以上静态合约 **7/7**、前端单元测试 **50/50**、通用浏览器检查 **13/13**、日志专项浏览器检查 **7/7** 均通过。浏览器使用 Chrome、新编译的真实嵌入资源和隔离临时配置，不修改已有预览或真实供应商配置。

- 通用脚本覆盖 1440px 与 390px 登录及九个路由，保存各页截图，并检查渠道三页弹窗的视口边界、水平尺寸、标题 / 标签 / 内容 / 页脚顺序与操作区可见性。
- 日志专项脚本使用本机上游及独立 SQLite 生成全部八字段的真实日志，检查六个详情标签、长内容预览、SVG 纯文本安全、滚动时标签不位移、两种宽度下无横向溢出。抽屉使用百分比宽度以适配根元素滚动条预留区，窄屏不预留稳定滚动条间隙，避免左缘负偏移。
- 抽屉 / 弹窗截图使用 viewport 模式，避免 full-page 截图把 top-layer 对话框放在打开前的文档滚动位置。
- 截图及 JSON 报告位于 `.artifacts/admin-ui-modern/` 与其 `logs/` 子目录。已检查登录、概览、插件、渠道编辑、日志设置与详情截图；零 JavaScript 异常、零 CSP 违规。

### 手工 QA 清单

- [ ] HTTPS 部署登录 / 注销、Secure Cookie、过期会话，密码错误与 429 限流。
- [ ] 桌面 1440px / 窄屏 390px，导航、横向表格、对话框、长模型名称。
- [ ] 只用键盘完成登录、跳转、表单、危险确认；检查焦点环 / Escape / 移动菜单。
- [ ] 断网 / 超时 / 503：显示重试入口，无模拟成功。
- [ ] 两个浏览器同时改配置，409 后不覆盖对方更改。
- [ ] 本地源 CRUD 与远程 CONFIG_URL 只读，无不可执行的操作按钮。
- [ ] 服务插件用真实受信任进程注册，schema 字段 / enum / secret / JSON，错误信任来源和凭据。
- [ ] 插件更新、启停、回滚，同时保留长请求，验证 leases / draining 真实变化。
- [ ] 字段 secret 留空保留，不回传脱敏文本；发现模型临时配置不持久化。
- [ ] 数据库开启时真实 Token 分组与筛选；禁用时清楚标明不可用；无逐条日志伪装。
- [x] settings 保存写入源配置，数据库开关均不影响重启保留；仅校验不写入，发布失败回滚，远程配置只读。

真实插件流量 / 排空、生产 HTTPS、数据库开启后的真实统计仍需对应部署与流量完成手工或插件专项集成测试。控制台没有模拟这些结果。
