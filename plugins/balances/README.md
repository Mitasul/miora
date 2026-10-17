# 可上传的余额插件

在仓库根目录执行 `python -B scripts/package_balance_plugins.py`，上传 `.artifacts/plugins` 中需要的 ZIP。
这些插件不替换渠道执行器，不需要另建渠道、复制账号或手动启动服务；安装后由现有托管插件管理器运行。

- `codex`：共享核心 OAuth，读取套餐和限额；不接管 refresh token。
- `novelai`：套餐、到期时间、订阅／购买 Anlas。
- `deepseek`：按币种显示余额与赠送／充值分项，支持原生 Chat 和 Anthropic 地址。
- `moonshot`：Kimi 开放平台钱包，不是 Coding Plan。
- `kimi-plan`：Coding Plan 限额及重置时间，不猜套餐档位，不伪装官方客户端。

官方地址由各自 manifest 自动匹配。反代在现有渠道中指定「余额插件」，Base URL 和 Key 继承该渠道；
可通过 `preferences.balance_plugin_config` 覆盖插件声明的配置。未安装／停用时如实提示，没有 Rust 内置兜底。

SDK 与实现说明见 `docs/channel-balances.md`、`sdk/python/README.md`。
包内仅有 manifest、插件源码和共享的无依赖映射工具，不携带用户凭据或 SDK 副本。

这是已完成离线模拟测试的实现，不代表已在真实账号上验收。
真实上传安装会创建依赖环境并启动插件进程；本次开发不执行该流程，不请求供应商，不修改现有运行数据。
