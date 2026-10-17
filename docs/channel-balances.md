# 渠道余额、套餐与额度插件

## 全部由插件执行

Rust 核心不再内置任何供应商余额接口、套餐名称或额度字段映射，包括原先的 Codex / NovelAI。
供应商 HTTP 请求与返回映射均在插件内；核心只保留管理员授权、能力选择、凭据解析、
插件版本租约、标准结果投影及持久化。没有未知渠道的 `/v1/usage` 探测。

余额插件独立挂到已有渠道，不把已有渠道改成新的 `plugin:<id>` 执行类型。
原有文本、图片、模型别名、调用 Key、分组与 OAuth 账号引用不变。
已有渠道插件自己声明的 `balance()` 仍可用，不强制拆分。

## 已提供的插件包

| 插件 ID | 用途 | 凭据 |
| --- | --- | --- |
| `balance-codex` | Codex 套餐、限额窗口、credits | 核心 Codex OAuth 的当前 AT 和对应账号 ID |
| `balance-novelai` | NovelAI 套餐、生效／到期、订阅及购买 Anlas | 渠道 API Key |
| `balance-deepseek` | DeepSeek 总余额、赠送余额、充值余额及币种 | 渠道 API Key |
| `balance-moonshot` | Kimi 开放平台可用、现金及代金券余额 | Moonshot 开放平台 Key |
| `balance-kimi-plan` | Kimi Coding Plan 订阅限额、窗口及重置时间 | Coding Plan Key |

源码在 `plugins/balances`。执行 `python -B scripts/package_balance_plugins.py` 后，
五个可上传 ZIP 位于 `.artifacts/plugins`。没有第三方 Python 依赖，SDK 仍由 Miora 安装时提供。
打包不安装、不启动插件，也不修改运行数据目录。

在后台「插件」上传需要的 ZIP；安装是受信任代码执行，需要管理员操作。
安装成功后，官方渠道按 manifest 的 `balance_targets` 自动匹配；无需创建另一条渠道或再填一份 Key。
自定义反向代理在现有渠道的「余额插件」里显式选择插件，不根据模型名猜供应商。
标准反代路径保留部署前缀；需要覆盖时可设置 `preferences.balance_plugin_config` 中插件声明的字段。
Moonshot 反代若不能从地址判断区域，币种保持未知，可显式提供 `currency: "CNY"` 或 `"USD"`。

Kimi 开放平台钱包和 Coding Plan 是不同产品、不同凭据及额度，不混用接口。
Coding Plan 支持详细的 `usage / limits`，及仅返回 `usages.*.used_ratio` 的响应格式；
两者并存时优先详细窗口，避免冲突数据重复展示。窗口时长来自响应，不把订阅额度固定叫周额度。
上游未提供套餐名称时不猜测 Pro／套餐档位，也不将配额冒充货币余额。

## 选择、认证与生命周期

- `preferences.balance_query: false` 关闭查询；`true` 只允许查询，不代表已有实现。
- 显式 `preferences.balance_plugin` 优先。`null` 或未设置时使用源渠道自己的余额钩子，
  然后匹配已安装且启用的插件 manifest。多个匹配时要求明确选择，不按遍历顺序随机决定。
- `balance_targets` 可声明 `engines`、`oauth_providers`、`origins` 和可选的 `path_prefix`。
  Origin 精确匹配，路径按段匹配；新增供应商无需修改 Rust 或前端。
- 未安装时不支持，已选插件丢失／停用／加载失败时明确报错，不回到其他供应商或核心内置查询。
- Codex 继续由核心共享账号、刷新锁与持久化；插件只收到同次账号选择的 AT、账号 ID 和稳定引用，
  不保存 RT、不读取核心文件、不刷新授权。旧网关是否仍在刷新必须由部署者确认，不保证跨进程共享锁。
- 插件 API Key 来自渠道选中的凭据；源渠道也是插件时复用它声明的上游认证配置。
  插件服务间 token 只认证核心到插件的连接，不作为供应商 token。
- 查询请求固定所用执行版本，更新后旧请求仍持有原快照；停用或回滚不改账号关联与渠道配置。
  未启动插件进程时，不能把这些离线租约测试当作真实热更新验收。
- 插件列表的使用渠道同时包含执行插件与独立余额插件的关联。更新和回滚复用现有配置兼容检查，
  使用与实际查询相同的渠道凭据和继承配置；不兼容时不发布新版，也不改写现有渠道。

## 展示与持久化

列表的「余额」按钮和编辑器的「获取余额」执行一次手动查询；打开编辑器只读保存结果。
没有后台轮询，不产生模型请求，不占调度账号，不改冷却或模型使用统计。
支持套餐、金额及单位、是否可用、剩余比例、多窗口、重置／到期时间及分项额度。
Unix 秒与 ISO 时间均可显示。DeepSeek 余额不可用与套餐未生效是不同状态。

多个账号独立显示，不汇总共享钱包或不同币种，每次最多查询八个不同凭据并显示省略数量。
未知值不伪装成 0；重置时间过去也不在本地恢复成 100%。
查询失败保留同配置、同账号的最近成功值，附失败状态和尝试时间。

- `GET /admin/api/channels/balances[?provider=渠道名]`：仅读持久快照。
- `POST /admin/api/channels/balances`，正文 `{"provider":"渠道名"}`：使用服务器凭据查询并保存。
  沿用后台管理员会话和 POST CSRF，不接受调用方传入上游 URL 或凭据。
- `GET /v1/channel-balances?provider=渠道名`：沿用平台管理员 Key 权限，使用相同插件选择及查询路径；
  不写后台快照，不提供历史日期或模型扣费聚合。

仍使用现有数据目录中的 `data/channel-balances.json`，支持 `MIORA_BALANCE_STORE_PATH`。
每个渠道仅存一份当前展示快照，没有 AT、RT、API Key、插件私密配置或原始上游响应。
重启后读取同一文件，不重新初始化配置，不创建备份或无限累积历史包。

## 接口依据与验证边界

映射依据 DeepSeek 的 `GET /user/balance` 文档、Moonshot 的账户余额 API 文档、
Kimi 官方 CLI 所使用的 `coding/v1/usages` 协议，以及本项目已有 Codex / NovelAI 映射，独立实现。
参考：https://api-docs.deepseek.com/api/get-user-balance 、https://platform.moonshot.cn/docs/api/balance 、
https://github.com/MoonshotAI/kimi-cli/blob/main/src/kimi_cli/ui/shell/usage.py 。
Kimi 比例格式仅有离线样例覆盖，不宣称其上游契约稳定；供应商拒绝非官方客户端时如实报错，不伪装身份。

离线测试使用模拟 HTTP，不监听端口、不启动插件／网关／浏览器、不联系付费上游。
覆盖格式、完整 SDK 调用、认证头、来源匹配、代理、错误、不重试、包内容、核心账号共享、
持久化重载、版本租约和后台交互。真实查询成功、真实安装和热更新／回滚／进程回收仍未验收。
