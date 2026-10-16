<p align="center"><img src="assets/branding/miora-logo.jpg" alt="Miora" width="200"></p>

<h1 align="center">Miora</h1>

<p align="center">把不同的 AI 服务放在一个地方管理。</p>

Miora 是用 Rust 编写的 AI API 网关，自带网页管理后台。你可以添加多个供应商的渠道，设置模型别名，给不同客户端分配调用 Key，并查看请求记录和用量。

后台和网关一起运行，不需要另外部署前端。

## 可以做什么

- **管理渠道和模型**：配置供应商地址与凭据，获取模型列表，设置对外名称。
- **分配访问权限**：创建调用 Key，通过分组控制可用渠道。
- **转发模型请求**：提供 Chat Completions、Responses、Messages 等接口，支持流式输出。具体能力取决于渠道和模型。
- **管理账号**：支持 Codex、Claude、Antigravity 的 OAuth 登录及凭据刷新。
- **测试与排查**：单独或批量测试模型，查看请求日志、错误和 Token 用量。
- **调整请求参数**：按渠道或模型设置参数，并预览处理后的请求。
- **扩展渠道**：通过插件接入其他服务，详见[插件文档](docs/channel-plugins.md)。

## 用 Docker 启动

需要先安装 Docker 和 Docker Compose。

```bash
git clone https://github.com/Mitasul/miora.git
cd miora
```

打开 `docker-compose.yml`，将 `MIORA_ADMIN_PASSWORD` 改成自己的密码，至少 12 个字节。然后运行：

```bash
docker compose pull
docker compose up -d
```

默认端口是 `8000`。打开 `http://localhost:8000/admin`，使用 `admin` 和刚设置的密码登录。部署在服务器上时，将 `localhost` 换成服务器地址。

首次进入后台后：

1. 添加渠道，填写地址和凭据。
2. 获取或手动添加模型，测试是否可用。
3. 创建调用 Key，分配所需分组。
4. 在客户端填写网关地址、调用 Key 和模型名称。

例如，使用 OpenAI 兼容接口的客户端通常将 Base URL 填为 `http://localhost:8000/v1`。后台登录密码和模型调用 Key 是两回事。

公网部署建议配置 HTTPS，并将 `MIORA_ADMIN_SECURE_COOKIE` 设为 `true`、`MIORA_ADMIN_ORIGIN` 设为实际访问地址。仓库默认 Compose 的 HTTP 设置适合初次试用。

## 数据与更新

默认 Compose 将数据保存在宿主机的 `./miora_data`，挂载到容器的 `/home/data`。配置、数据库和插件数据应保留在这个目录中，而不是只放在容器内。

更新已发布的镜像：

```bash
docker compose pull
docker compose up -d
```

更新时继续使用原来的数据目录和部署配置，不要用仓库中的默认密码或挂载路径覆盖自己的设置。容器重建会短暂中断服务，也可能需要重新登录后台。涉及数据结构变化时，请先查看版本说明。

镜像地址：`ghcr.io/mitasul/miora`。可使用 `latest`，也可以固定到已发布的提交标签。

## 插件

插件用于接入有特殊认证、参数或返回格式的渠道。安装方式、支持范围和开发示例见[渠道插件文档](docs/channel-plugins.md)及 [`examples/plugins`](examples/plugins)。

不同供应商的接口不一定能无损转换。接入后应测试实际需要的能力，例如工具调用、图片和流式响应，而不只是检查模型列表。

## 从源码构建

需要 Rust 工具链和 C 编译环境，具体版本要求见 `Cargo.toml`。

```bash
cargo build --release --locked
```

生成的程序为 `target/release/uni-api-front`。运行前需要配置管理员凭据及配置文件路径，见[管理员配置](docs/admin.md)。

## 文档

- [管理员配置](docs/admin.md)
- [渠道插件](docs/channel-plugins.md)
- [请求参数设置](docs/parameter-policy.md)
- [请求日志与清理](docs/request-logs.md)
- [功能与限制](docs/gateway-features.md)
- [管理界面](docs/admin-ui.md)

## 致谢

Miora 基于 [uni-api](https://github.com/yym68686/uni-api) 开发，部分功能与界面设计参考了 [Zoaholic](https://github.com/HCPTangHY/Zoaholic)。感谢这些项目的作者和贡献者。

## 许可证

[Apache License 2.0](LICENSE)。第三方依赖说明见[依赖许可证](docs/dependency-licenses.md)。
