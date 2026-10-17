# Miora 托管 Python 渠道 SDK v1

这是独立实现的 Miora 原生插件 SDK。已有 Rust 渠道继续由 Rust 执行；不需要为认证头或几行历史修饰重新实现整个网关。

## 三种扩展

| 类型 | 安装与执行 |
| --- | --- |
| 内置适配器 | 随 Miora 发布，复用现有 Rust 认证、调度、协议转换、统计 |
| 托管脚本 | 管理员上传 ZIP；Miora 创建 venv、安装依赖、启动独立进程、检查健康状态并回收旧执行版本 |
| 独立服务 | 保留 `.miora.json`；服务由操作者部署，Miora 不接管远端进程 |

请求头拦截器仍是独立职责，不是渠道插件。托管插件需要宿主已安装 Python **3.10+**，可用 `MIORA_PLUGIN_PYTHON` 指定解释器绝对路径。无需手动启动插件服务或分配端口。Windows 的宿主启动命令使用 `CREATE_NO_WINDOW`。

## 制作与上传

ZIP 根目录必须直接包含：

```text
manifest.json
plugin.py
requirements.txt    可选
```

`examples/plugins/managed-openai/` 是不带凭据、不依赖第三方包的示例。在该示例目录执行：

```powershell
Compress-Archive -LiteralPath .\manifest.json,.\plugin.py -DestinationPath .\managed-openai.zip
```

后台“插件管理 → 上传插件”选择 ZIP，先看清单、版本变化、依赖、权限声明和配置字段，再确认可信代码并安装。**预览只读取包，不执行插件代码、不安装依赖。** 安装成功后选择“创建渠道”，填写该实例的地址、凭据，获取模型并保存。一个插件可创建多个独立渠道。安装阶段不要求填写渠道配置。

插件和依赖都是受信任宿主代码，独立进程不是安全沙箱。权限声明仅供管理员审阅，不是操作系统权限限制。ZIP 解包限制路径、链接、宿主保留目录、文件数量和展开体积，不能靠包内路径覆盖包目录外的文件；这不限制确认安装后受信任 Python 代码本身的宿主权限。

上传 ZIP 最多 16 MiB，展开最多 64 MiB／1024 项；清单最多 256 KiB。没有自定义包哈希、签名或摘要验证。安装依赖使用当前解释器的标准 venv/pip，不自动检查或执行线上新版本。

## 复用基础适配器

```python
import json
import pathlib
from miora_plugin import OpenAIChat, serve

class Vendor(OpenAIChat):
    def headers(self, context):
        headers = super().headers(context)
        headers["x-vendor-project"] = context.config["project"]
        return headers

manifest = json.loads(pathlib.Path("manifest.json").read_text(encoding="utf-8"))
serve(Vendor(manifest))
```

声明 `project` 配置字段即可，不需要改前端或 Rust。`OpenAIChat`、`Responses`、`Claude` 复用 HTTP、认证、超时、模型发现、JSON/SSE 转发。它们默认传递自己声明支持的原生端点，不代表插件内自动实现任意跨协议矩阵。已有 Rust 协议适配仍保留。

可覆盖 `headers(context)`、`prepare(context)`、`invoke(context)`、`models(context)`。完全自定义响应使用 `Response(status, headers, body)`；body 支持 bytes、带 `read()`／`read1()` 的文件或 HTTP 流、字节迭代器。默认转发保留上游 SSE，不制造结束事件。

`sse_events(lines)` 解析 SSE，`standard_events(event, payload)` 将 Chat／Responses／Claude 转成 text、reasoning、tool_start、tool_delta、usage、done、error 事件；工具参数仍是增量，不假装已经是完整 JSON。usage 保留原始 detail 对象。断流不补造完成。下游断开在写失败时设置 `context.cancelled` 并关闭上游；正在阻塞的上游读取仍受配置超时约束，**不是即时抢占取消**。自定义迭代器应检查该事件并在 close 时释放资源。

## 多模态 HTTP 与受控资源

`examples/plugins/managed-multimodal/` 演示使用同一基础类接收四种请求体。示例地址是占位符，必须配置供应商地址和支持的端点，不代表已通过任何供应商验收。

供应商原生客户端应使用后文的 `NativeProxy` 模式；此处的通用协议模式仍会应用其既有模型映射和参数策略。

清单以 `capabilities.request_formats` 显式启用 `json`、`form`（URL 编码）、`multipart`、`binary`；省略时仍仅接受 JSON，旧独立服务不会被偷偷发送二进制信封。现有图片生成、图片编辑、语音转录等网关端点继续使用既有模型别名、调用 Key、分组与调度。

```python
self.open_http(context, "/v1/render", {"prompt": "draw", "vendorStyle": "ink"})
self.open_http(context, "/v1/render", form={"prompt": "draw", "seed": 42})
self.open_http(context, "/v1/render", multipart={"prompt": "edit", "image": image_resource})
self.open_http(context, "/v1/render", binary=image_resource, method="PUT")
```

四种 body 参数互斥。`headers=` 可声明供应商媒体类型／额外头，`method=`、`timeout=` 可覆盖本次 HTTP 行为；不再要求每个插件重写 JSON、表单、文件分段发送。二进制可用 bytes 或 `Resource`，**字符串文件路径不会被当作可读取文件**。默认 `Channel.invoke` 按入站格式选择编码，表单中的模型别名替换为调度后的真实模型。

`context.resources` 是单次执行的资源集合：

- `add(bytes_or_reader_or_chunks, filename=..., content_type=...)` 接收数据并返回 `Resource`；资源总计最多 128 MiB／128 项。
- `get(resource_id_or_metadata)` 仅接受本次集合持有的引用，不读取任意宿主路径或其他请求的资源。
- `Resource.metadata()` 只含 ID、文件名、媒体类型和大小，不含宿主路径；`open()`／`chunks()` 按需读取。
- `from_data_url(...)` 将显式提供的 base64 图片转换为资源；不会自动下载任意客户端 URL。
- `context.body_resource` 是原始非 JSON 请求体；`context.parameters` 解析 URL 编码／multipart，文件字段为 `Resource`，重复字段保留为列表。
- `download(context, endpoint, filename=...)` 通过渠道 HTTP 客户端下载到同一受限集合，可处理图片或 ZIP 等二进制；不自动解压归档。

资源按需落临时盘，不把文件编码进 JSON。宿主沿用已有请求 spool，非 JSON 输入通过带 JSON 前缀长度的认证内部通道流式交给插件，SDK 再按需解析 multipart。当前 SDK 单次内部请求最多 64 MiB（含信封），文本表单 1 MiB／128 字段，multipart 单个文本字段 1 MiB／头部 16 KiB。宿主已有请求、响应、磁盘及超时限制仍可能更低。临时资源在响应体关闭后清理，不保留旧包或资源备份，也不计算哈希。

这不是跨请求文件服务器：临时 ID 不能在请求完成后用 URL 取回。需要返回文件时直接返回 `Response.resource(resource)`，ZIP 可设置 `attachment=True`；需要 JSON 时使用 `Response.json(value, status=...)`。原始 `Response` 也可直接转发图片、归档或 SSE 流，不将二进制假装成 JSON。它不是进程沙箱，受信任插件代码仍拥有宿主进程权限。

URL 编码和 multipart 的列表值按重复字段发送，布尔及对象值采用 JSON 文本而不是 Python 的 `True`／字典表示；如果供应商要求单字段 JSON 数组，插件可在 `prepare` 中显式 `json.dumps`。

### 专有参数与动态表单

`config_schema` 仍描述渠道级持久配置；新增 `request_schema` 描述客户端请求参数，使用相同字段形式：`key`、`label`、`type`、`required`、`default`、`enum`。类型支持 string、integer、number、boolean、object、array，以及上传文件用的 `resource`。

网关不把声明字段交给通用参数过滤器删除或改写；原始值传给插件，参数预览标注 `plugin_schema`。SDK 对 `context.parameters` 按声明检查类型／枚举／必填，应用默认值，转换表单及已声明查询参数的文本值；失败明确返回 400，不丢字段或假装调用成功。未知业务字段也保留，由插件自己的 `prepare`／`invoke` 决定如何转换。

已声明的 URL 查询参数会单独传给插件；未声明查询参数不透传，尤其不顺带转发客户端认证头、Cookie 或网关调用 Key。优先级为 **body／表单 > 已声明 query > 清单默认值**，模型始终采用宿主选定的模型。通用 JSON 参数策略不能用于重写原始非 JSON 文件内容；这类变换由插件负责。

安装后渠道表单、JSON 请求预览根据声明生成枚举、数值、对象等控件，不需要为新插件改前端。资源字段使用 multipart 实际上传，JSON 离线预览不会伪造一个可用文件。上传更新的预览同时展示专有参数及请求格式变化。

### 异步任务轮询

SDK 提供可覆盖供应商状态映射的轮询工具，不强制某家供应商的任务协议：

```python
submitted = self.read_json(self.open_http(context, "/v1/render", self.prepare(context)))
task_path = "/v1/tasks/" + urllib.parse.quote(submitted["id"], safe="")
finished = self.poll_task(
    context, task_path,
    status=lambda result: result["task_status"],
    completed=("SUCCEEDED",), failed=("FAILED", "CANCELED"),
    interval=1, timeout=120,
)
archive = self.download(context, task_path + "/archive", filename="result.zip")
return Response.resource(archive, attachment=True)
```

示例需 `import urllib.parse`。`poll_task` 使用有限间隔与超时，检查协作取消，失败／取消状态不伪造成功；`on_update=` 可接收每次结果。该工具是当前请求内的轮询，不是持久任务队列，不承诺网关重启后自动续跑。下游断开只有在写入失败时被观察，阻塞 HTTP 读取仍依赖超时，不宣称轮询中即时感知断开或自动取消供应商远端任务。

## 表单与凭据

Manifest 支持 `name`、`author`、`description`、`version`、`api_version`、`sdk_version`、`changelog`、`permissions`、`config_schema`、`auth`、`capabilities`、`models`。

配置字段包括类型、默认值、枚举、必填和 `secret`。认证支持 none、bearer、api_key；敏感字段不允许在清单默认值内携带凭据。实例配置复用现有渠道持久化和管理端脱敏机制。配置边界沿用宿主 schema，不另加一套每帧验证或重复请求验证。

`context.credentials.transaction()` 提供按插件及渠道隔离的持久化字典。它保存在已有 `MIORA_PLUGIN_STORE_DIR` 下的 `credentials/<plugin-id>/accounts.json`，不在某个将被清理的执行版本目录中；更新、回滚共用同一份凭据。操作系统文件锁跨进程串行化读改写，避免旧版排空与新版同时刷新轮换 RT。内容不变时不重写或 fsync。该锁按插件串行化，通用 SDK 的慢刷新可能阻塞同插件其他账号；高频渠道优先继续使用原生 Rust 实现。

凭据是明文文件而非加密保险库。Unix 新文件权限为 0600，Windows 继承数据目录 ACL，部署者需要限制数据目录访问。凭据不放在前端本地存储中；SDK 状态接口只返回连接／可刷新／到期状态，不返回 token。插件是可信代码，作者仍须避免自行把凭据打印到日志。

## OAuth 扩展

在 `capabilities` 声明 `"oauth": true`，后台渠道表单自动出现获取授权链接、兑换授权码、刷新和状态按钮。必须先填写稳定的渠道名称；模型发现和执行使用同一渠道名读取账号，渠道名不是临时调试请求 ID。

```python
from miora_plugin import OpenAIChat, OAuth

class Vendor(OpenAIChat):
    oauth = OAuth(
        "https://vendor.example/oauth/authorize",
        "https://vendor.example/oauth/token",
        "registered-client-id",
        ["models.read", "inference"],
    )
```

此模式通常声明 `auth.type: none`，由 OAuth 扩展添加上游 Authorization。配置 schema 声明 `redirect_uri`，必要时声明敏感的 `client_secret`。宿主只通过认证后的内部 `/oauth` 信封调用扩展，插件自己持久化与刷新 token。`OAuth` 提供普通授权码流程、state 绑定及 refresh token 轮换保存；可以覆盖 authorize、exchange、token_request、access_token 适配供应商。基础类不实现 PKCE、设备码或供应商专用登录，也不暗中为此添加哈希计算；需要这些协议的供应商必须提供相应扩展，不能冒充现成兼容。Codex 等已有原生 OAuth 不迁到此基础类。

## 更新、回滚与清理

- 发布版本 `version` 由作者提供；API/SDK 兼容版本当前均为 1；内部 `revision` 仅用于冲突检测与定位执行版本，三者分开展示。
- 上传相同 ID 的新版：准备新 venv／依赖，启动并健康检查成功后才发布。失败不替换旧版；安装日志和错误可在后台查看。
- 新请求固定新版快照，在途请求继续持有旧版进程。旧版租约释放后退出。禁用阻止新请求，不切断在途流。
- `config_aliases: {"new_name": "old_name"}` 支持字段改名投影；新增字段可给默认值。配置与凭据不因更新清空。无法投影的类型或必填变化拒绝发布并指出不兼容渠道，不运行任意配置迁移代码。
- 回滚使用目标执行版本及其默认配置检查**当前渠道实例**是否兼容；兼容才切换。不会把用户渠道配置倒回旧快照。不可逆配置修改可能导致旧版不再可回滚。
- 常态保留至多 3 个可用执行版本；在途版本不删除，租约释放后在下一次更新中清理。100 个仍占用的历史版本是已有上限。旧持久化 generation 文件在发布新完整状态后清理，不额外生成配置或数据库备份。进程日志最多 200 块／400 KiB，内存安装记录最多 128 项。
- 独立服务更新必须使用另一个不可变执行端点，并继续保留旧服务；宿主拒绝仅用同一个可变 URL 换描述来承诺无损回滚。健康检查会核对目标发布版本，执行同时发送版本头、内部修订头及版本化信封。远端程序的部署与保留仍由操作者负责。

## 供应商原生协议认证代理

`examples/plugins/managed-native/` 提供独立的原生协议示例。将清单和入口脚本打成 ZIP 上传后，使用插件创建渠道并填写上游地址、凭据和模型。不修改 Rust 或前端，不要求部署额外服务。示例的 `/api/*` 是演示协议，不能直接当作某个真实供应商的兼容适配器。

```python
from miora_plugin import NativeProxy, serve
serve(NativeProxy(manifest))
```

客户端把原供应商域名换成 `https://你的Miora/native/<URL编码的渠道名>`，把供应商凭据换成有权访问该渠道的 **Miora 调用 Key**。保留原客户端 Base URL 的 API 路径后缀：例如原 Base URL 是 `https://vendor.example/v1`，新地址应为 `https://你的Miora/native/my-channel/v1`。客户端仍发送供应商原生请求，不需要改成 Chat Completions。渠道名称改变后相应 URL 也改变。

清单通过 `native_proxy` 声明原生入口，`capabilities.endpoints` 可以为空；同一插件也可同时声明通用协议端点。一个原生入口只访问所选渠道的配置地址，不能通过请求指定上游域名。原生流不走通用空响应重试、图片下载或协议转换，不重写 model、专有参数、JSON 空白、表单或 multipart 边界。

```json
{
  "native_proxy": {
    "client_auth": {"type": "header", "name": "x-vendor-key"},
    "routes": [
      {"path": "/v1/generate", "methods": ["POST"], "model": {"source": "body", "pointer": "/model"}},
      {"path": "/v1/models/{model}:generateContent", "methods": ["POST"], "model": {"source": "path"}},
      {"path": "/v1/uploads", "methods": ["POST"], "model": {"source": "fixed", "value": "vendor-files"}}
    ]
  }
}
```

- `client_auth` 选择 `bearer`、`header` + `name` 或 `query` + `name`。它规定客户端原本使用的认证位置，不要求额外发送 Miora 专用头。被选中的凭据用于现有授权、分组、模型权限和限流；不能用其他诊断／BYOK 头越过渠道选择。
- `auth` 仍声明**上游**认证，使用托管渠道凭据；支持 `bearer`、`api_key` + `header`、`query` + `query`，以及现有 `oauth` 扩展。客户端调用凭据、Cookie、转发头和网关控制头不进入上游请求。Query 凭据在进入请求日志前移除；其余 query 保持原编码、顺序和重复项。
- 路由 `path` 支持精确路径、末尾 `*` 和单个 `{model}`；`methods` 显式列出允许的方法。每条路由声明模型授权来源：`body` + JSON `pointer`、`form` + `field`（表单／multipart 文本字段）、`query` + `name`、`path`，或 `fixed` + `value`。文件、任务等没有模型的接口用单独的固定权限模型；不要用固定模型规则隐藏实际存在的生成模型。
- 从原生请求取出的是供应商真实模型名，反向对应已授权的渠道模型别名；别名只用于权限、限流和日志，不改写出站正文。原有通用接口的别名转换照常工作。固定权限模型也必须加入渠道模型并授予调用 Key，不能因为是文件／任务接口就绕开授权。渠道的禁用端点和请求规则继续按原生业务路径匹配，不会因 `/native/渠道` 前缀而失效。
- 请求体走已有受控资源 RPC；`context.body_resource` 提供原始字节，`request.endpoint`、`query`、`headers` 提供业务 HTTP 元数据。原生模式不自动访问 `context.parameters`、过滤字段或套用 `request_schema`；需要专有参数转换时由插件显式实现。`headers()`、`url()`、`invoke()`、`models()`、OAuth 接口仍可覆盖。
- JSON、表单、multipart、图片／文件二进制与 SSE 使用同一条原生通路；响应保留 HTTP 状态、业务头、重复响应头、Content-Encoding 和原始字节，包括非 2xx、重定向和供应商错误事件。只移除逐跳／连接指定头及重新分帧所需的 Content-Length；不自动跟随重定向。响应中的相对 URL 不改写，客户端需在同一个原生 Base URL 下解析；绝对供应商资源地址仍是原地址。
- 原生模式不自动重试、跨渠道回落或对冲，避免重复创建任务或上传文件。需要账号一致性的文件／异步任务，使用每个渠道一个上游账号；通用任意路径目前不提供跨 Key 池的任务账号绑定。WebSocket／HTTP Upgrade 不属于此 HTTP 代理能力。
- 压缩字节不会被原生入口解压后再转发。`body` 模型提取仅为授权读取 JSON 视图，复用当前 identity／zstd 解码能力；其他 Content-Encoding 应将模型授权放在 path、query 或适用的固定接口规则中，不能冒充已支持 gzip 正文模型提取。
- 用量是旁路观察：识别现有 Chat／Responses／Claude／Gemini usage 字段和缓存细项，不改变输出。未声明的供应商用量格式保持未知，不伪造 0 或完成事件。观察缓冲最多 1 MiB，超过后只停止观察，不截断转发；未知原生协议的 HTTP 200／流结束不等于完成了业务兼容验收。

## 验证范围

`tests/plugins/test_managed_sdk.py` 是不监听端口、不启动插件、不访问上游的解包、SSE、凭据和模拟 OAuth 测试；Rust `migration_compat` 测试覆盖协议出站字段、持久化重载和版本租约状态，前端模块测试覆盖 ZIP 上传交互。它们不是一次真实 pip 安装、进程热更新或上游账号验收。生产安装／热更新需在操作者明确允许启动插件后再验收。

Rust `native_proxy` 定向测试另覆盖渠道／模型／分组权限、凭据位置、原始 query／正文／响应、非 2xx 和重定向、SSE 用量、软错误和取消；Python 测试使用模拟传输检查多格式原生字节、认证覆盖和 OAuth 钩子，不启动服务、不调用供应商。
