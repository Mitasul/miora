# ADR 0001：Miora 动态渠道插件与阶段性接口

状态：独立进程选型已采用；下文严格区分现有 v1、第一轮样例及未来接口草案。

## 1. 决策

先使用**独立插件进程/服务 + HTTP/JSON/SSE**。保留 Rust 主程序现有路由、协议转换、超时与观测机制，不采用 Python 动态 import，不使用本机 Rust 动态库 ABI 充当稳定插件协议。

| 维度 | 独立进程/服务 | 嵌入脚本运行时（例如 QuickJS） |
| --- | --- | --- |
| 流式传输 | HTTP/SSE 已有互操作性，可渐进转发，背压由读写链路承担 | 需要宿主流桥接、异步调度和内存管理；不能只执行一个同步脚本函数 |
| 取消 | 核心取消服务连接；插件必须继续取消其上游工作 | 需中断脚本和宿主 I/O，不能仅停止 JS 执行 |
| 超时 | 核心请求策略与插件 I/O 共同约束，不引入隐藏的固定短超时 | 需分别控制脚本 CPU 和宿主网络等待 |
| OAuth | 服务可管理刷新状态及外部回调；宿主提供受控认证能力接口 | 同样需要安全存储、回调和异步刷新宿主 API |
| 部署 | 需要单独进程/容器；可复用不同语言运行时 | 分发方便，但增加宿主引擎、绑定与平台维护成本 |
| 故障隔离 | 插件崩溃通常不直接带走 Rust 主进程 | 脚本引擎/FFI 问题在宿主内，需额外资源与故障隔离 |
| 权限隔离 | **不是自动沙箱**，仍需用户/容器/网络/文件权限 | **也不是自动安全**，原生宿主函数可能绕过 VM 限额 |
| 许可证与维护 | 本轮只用 Python 标准库写独立样例，不新增 Rust 依赖 | 引擎及绑定依赖须单独核对许可证后再引入 |

未来可评估托管 Node/Python 进程或嵌入脚本引擎，但不因此更换现有 wire contract。语言和进程启动方式是部署细节，不能成为渠道供应商分支。

## 2. 四个实体的目标模型

以下是**后续配置结构草案，不是当前已支持的导入格式**：

```yaml
channel_types:
  - id: openai.chat
    implementation: builtin
  - id: sample.header-chat
    implementation: plugin
    plugin_id: header-chat
channels:
  - id: channel-a
    name: 主力兼容上游
    type_id: openai.chat
    enabled: true
    connection: {}
    credentials: []
    models: []
    parameter_policy: {}
    routing: {}
    processing_pipeline: []
```

- 稳定实例 ID 与可编辑名称分离；多个实例可使用同一类型。
- 插件可以贡献渠道类型；供应商品牌和默认地址通常只是类型的配置模板。
- 普通兼容服务无需新增插件。外部反代可直接作为上游，不搬入其全部实现。
- 处理插件使用独立类型和配置结构，按渠道实例选用和排序。
- 迁移需保留旧 `provider`/`engine` 引用，不在第一轮批量重写已保存渠道。

## 3. 当前 v1 wire contract

现有实现见 `src/plugins/manifest.rs`、`registry.rs`、`runtime.rs` 与 `docs/channel-plugins.md`。

### 3.1 已支持的 manifest

```json
{
  "id": "header-chat",
  "name": "Header Chat example",
  "version": "1.0.0",
  "api_version": 1,
  "description": "独立真实转发样例",
  "config_schema": [
    {"key":"base_url","label":"上游地址","type":"string","required":true,"secret":false},
    {"key":"api_key","label":"供应商凭据","type":"string","required":true,"secret":true}
  ],
  "auth": {"type":"api_key","config_key":"api_key","header":"X-Sample-Api-Key"},
  "capabilities": {
    "endpoints":["/v1/chat/completions"],
    "streaming":true,
    "model_discovery":true
  },
  "models":[]
}
```

这个结构只表示当前能力。宿主目前的动态 manifest 不接受任意新增字段；不要直接把未来 hook/schema 字段发给 v1 管理 API。

### 3.2 控制与数据通道

| 接口 | 已有语义 |
| --- | --- |
| `GET /health` | 独立服务认证；返回实际加载的 id/version/api_version。候选健康失败，不发布新版本。 |
| `POST /models` | 收到选定插件版本、渠道配置与认证声明，返回 `models` 数组。 |
| `POST /execute` | 收到选定版本和已路由请求，返回对应下游协议的真实 JSON/SSE。 |

`/execute` 的关键字段：

```json
{
  "api_version":1,
  "plugin":{"id":"header-chat","version":"1.0.0","revision":1},
  "request":{
    "id":"request-id",
    "method":"POST",
    "endpoint":"/v1/chat/completions",
    "requested_model":"public-alias",
    "model":"upstream-model",
    "stream":true,
    "body":{"model":"upstream-model","messages":[{"role":"user","content":"hello"}],"stream":true}
  },
  "channel":{"name":"example-channel","config":{}},
  "authentication":{"type":"api_key","config_key":"api_key","header":"X-Sample-Api-Key"}
}
```

客户端请求名和上游发送名已分开。上游响应中的模型名必须原样观察，不能为了看起来一致而默默替换；三种名字的完整持久化和测试 UI 仍属于后续工作。

控制通道 Bearer 与供应商凭据分离。管理员 Cookie、CSRF token、调用者访问 Key 不进入插件请求信封。插件知道它被授权使用的渠道凭据，不因此拥有管理 API 权限。

### 3.3 版本、取消和完成语义

- 配置修改用管理 revision；执行使用不可变版本。两者不是同一个概念。
- 新版本预检成功后 CAS 切换活动引用；预检等待期间不持有注册表写锁。
- 请求在入口固定版本快照，重试/hedge 不跳到新插件版本；响应 Body 持有租约到完成、出错或取消。
- 更新不原地覆盖旧服务实现。真实不同版本使用不同进程或不可变部署端点。
- 停用阻止新请求，不杀掉旧流。
- 核心断开插件连接后，插件必须关闭其上游；本轮样例使用客户端断连监视处理上游静默情形。
- 插件不得补造 `[DONE]`、完成事件、usage 或 OK 来掩盖断流。HTTP 200、无 usage、输出 token 为 0 均不是独立的失败/成功判据。
- 本轮逐块转发用于证明流、取消及背压链路；不等于所有协议、工具多回合与运行时都已经验收。

## 4. 未来能力草案：不在本轮冒充已实现

未来 manifest 需要版本化扩展：

```json
{
  "channel_types":[{
    "id":"vendor.special-chat",
    "protocol_profile":"openai.chat",
    "default_base_url":"https://provider.example/v1",
    "authentication":["api_key"],
    "capabilities":{
      "prepare_request":true,
      "modify_passthrough":false,
      "custom_transport":false,
      "parse_response":false,
      "parse_stream":false,
      "discover_models":true,
      "refresh_auth":false,
      "query_balance":false
    }
  }]
}
```

`false` 的能力不能在 UI 中伪装可用。声明标准协议 profile 后，应尽可能使用宿主已有实现；只有差异才经过可选 hook。当前 v1 整体 `/execute` 作为兼容服务模式保留，不强行一次拆掉两条生产执行器。

后续接口职责草案：

- 请求构造返回结构化 `RequestPlan`，不能返回可执行 shell；包含目标、方法、头、正文和协议 profile。
- 流处理使用有界事件/字节通道并提供取消，不能将整条流攒成单个 JSON RPC。
- OAuth 授权开始/回调完成/刷新/解绑是认证能力；refresh token 不返回前端。
- 额度接口返回结构化余额与更新时间，不伪造实时性。
- 处理插件输入为请求/响应上下文和有限补丁输出，独立于渠道类型；每渠道保存顺序与配置。服务模式必须区分宿主到插件的控制通道头与插件到供应商的请求头，不能把修改前者误报为已经修饰后者。
- 复杂 UI 必须使用独立来源或严格 sandbox 的扩展容器，通过窄消息接口交互；不给任意插件 JS 管理端 Cookie 或完整 API 权限。

以上需在后续接口版本中逐项落实，不能向现有 v1 直接增塞字段而破坏 `deny_unknown_fields`。

## 5. 安装、托管与卸载草案

当前 UI 的添加操作是“注册已经运行的服务”，不是上传代码包安装。现有 registry 也不是进程管理器。

后续安装管理需要分清：

1. 管理员明确授予代码执行能力；普通访问 Key 无此权限。
2. 暂存并解析包和入口，防止路径穿越；不在保存渠道配置时执行安装或重启。
3. 启动候选进程，核对宿主接口兼容与健康。
4. 原子激活或保留旧版，并显示失败原因。
5. 卸载前检查所有保存渠道的类型引用与在途租约；有引用时返回冲突并列出引用渠道，不直接强杀。
6. 旧版本排空后才允许清理其进程与文件。

本轮不引入代码上传、包签名或文件校验框架；不把它们包装成现成能力。外部服务的升级部署由管理员管理，核心不能阻止外部工具误杀旧进程。

## 6. 参数策略契约草案

阶段二按下列固定顺序接入：

```text
协议适配器生成请求
  → 渠道默认值（仅补客户端/协议未提供的字段）
  → 渠道强制覆盖与删除
  → 精确模型规则覆盖与删除
  → 本渠道有序处理插件
  → 该协议最终约束
```

### 三态

| 状态 | 语义 |
| --- | --- |
| 继承 | 本层不写字段；保留客户端/协议或前层值，若都没有则不发送。 |
| 设置 | 写入指定值；0、false、空字符串、null 是值，不等同未设置。 |
| 删除 | 独立 remove 操作，删除字段；不把 null 当删除。 |

例子是策略草案，不是当前导入 JSON：

```json
{
  "defaults":{"temperature":0.2},
  "channel":{"set":{"stream_options":{"include_usage":true}},"remove":["/seed"]},
  "models":[{
    "match":{"requested_model":"public-model"},
    "set":{"temperature":0},
    "remove":["/top_p"]
  }]
}
```

- 对象递归合并；数组整体替换；客户端显式 null 不算缺省。
- 同层 set/remove 不允许针对同一路径互相矛盾；更具体的模型层可以重设前层删除的值。
- 草案使用明确路径；旧 `__remove__` 点路径和特殊字段规则需迁移兼容，不暗中改其含义。
- 模型规则首先按客户端请求名精确匹配；若后续允许按上游名匹配，需独立 selector，不能悄悄混用。
- UI 只编辑改变的路径，未知 JSON 字段保留。普通表单与高级 JSON 往返必须有测试。
- 采样参数没有设置就不发送。token 上限由协议能力映射，只生成该适配器支持的一个字段。
- effort、thinking、service_tier/speed 等由具体类型声明；未知能力不显示为可用选项。
- 系统提示的前置/后置/替换是明确策略，由适配器按协议表示执行，预览展示结果，不能无提示覆盖用户提示。

## 7. 最小可运行交付

目录：`examples/plugins/header-chat/`。

目的不是让每个兼容服务都写插件，而是用一个有自定义鉴权头的简单差异，证明渠道插件同时负责模型发现、认证和真实请求执行。

```text
测试客户端
  → Miora 公共 API / 配置 / 路由 / 日志
  → 固定版本的独立 header-chat 服务
  → 配置的真实 HTTP 上游
  → 原始 JSON 或逐块 SSE 返回
```

自动测试的最后一层是明确标注的本地上游 fixture，断言收到的 path/body/auth，以及实际返回的流字节。插件本身不实现“看到提示就返回 OK”的假成功。没有线上授权时，不据此声称真实供应商生产认证成功。

本轮不重启用户现有预览；样例和测试只启动隔离进程。运行说明、结果及未覆盖项以样例 README 和交付回复为准。
