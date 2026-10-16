# 第三方依赖许可证声明清单

核对日期：2026-10-13。范围：当前 Cargo.lock 解析得到的全部目标平台依赖（含构建和未必在当前平台链接的包），不进行依赖升级。

## 项目与参考边界

- Miora 保留 uni-api 上游归属及根目录 Apache-2.0 LICENSE；Cargo 包元数据也声明 Apache-2.0。
- Zoaholic 的 GitHub license API 返回 GPL-3.0。本轮只查询该元数据，没有读取或复制其源码、组件、测试或文档，也没有将其作为依赖。
- 本轮新增样例为独立编写的 Python 标准库程序，沿用本仓库 Apache-2.0；未添加 Rust 或浏览器运行时依赖。
- 前端使用项目自有模块和本地 SVG；CSS 字体名称不等于打包分发字体文件。
- 2026-10-16 新增 AI 品牌 SVG，取自 `@lobehub/icons-static-svg@1.95.1`，仅分发图像，不引入 JavaScript 包。上游 MIT 许可证原文和来源说明分别保留在 `assets/admin/ai-icons/LICENSE` 与 `assets/admin/ai-icons/SOURCE.md`；许可证也嵌入前端静态资源。

## 核对层次与结论

下表逐包记录实际解析版本和上游声明的许可证表达式。**这是一份许可证声明核对表，不冒充完整发布制品法律审查，也不是替代各许可证原文的 THIRD_PARTY_NOTICES。** 当前包均提供许可证声明；声明中未发现必须选择 GPL/AGPL 的依赖。

- `OR` 表示可选择许可分支，不要求同时选择所有分支；例如 r-efi 可采用 MIT 或 Apache-2.0，无需采用其可选 LGPL 分支。
- `AND` 的各项义务须同时满足。ring 的 ISC、matchit/encoding_rs 的 BSD-3-Clause、unicode-ident 的 Unicode-3.0 不能因项目自身采用 Apache-2.0 而忽略。
- `MIT/Apache-2.0` 是包中保留的历史写法；发布时以该包附带的许可文本确认选择，不将斜杠自动当成 AND。
- `CDLA-Permissive-2.0` 适用于 webpki-roots 的数据分发；不能把它改记为 MIT。
- `BSL-1.0` 是 Boost Software License 1.0，不能与其他名称相似的许可证混淆。

正式发布仍需：收集所选许可要求的许可证/版权通知；审查 ring、zstd-sys、libsqlite3-sys 等捆绑的 C/汇编/数据组件声明；核对打包进制品的资源来源；按实际目标区分链接依赖和开发工具。当前清单不把这些未完成项标为通过。

`.tools` 中的本地 Rust/GCC/Playwright 等仅用于开发，未纳入产品包。若将 Python 解释器、工具链或浏览器随产品再分发，必须另行保留其许可，不能只引用本表。

## 清单

- 直接依赖：27 个（包含 Windows 条件依赖）。
- 全部解析的第三方包：225 个；每个包和版本独立列出。

| 包 | 版本 | 依赖层级 | 声明的许可证 | 核对说明 |
| --- | --- | --- | --- | --- |
| [addr2line](https://crates.io/crates/addr2line/0.25.1) | 0.25.1 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [adler2](https://crates.io/crates/adler2/2.0.1) | 2.0.1 | 间接/构建/目标平台 | `0BSD OR MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [ahash](https://crates.io/crates/ahash/0.8.12) | 0.8.12 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [async-trait](https://crates.io/crates/async-trait/0.1.92) | 0.1.92 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [atomic-waker](https://crates.io/crates/atomic-waker/1.1.2) | 1.1.2 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [axum](https://crates.io/crates/axum/0.8.4) | 0.8.4 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [axum-core](https://crates.io/crates/axum-core/0.5.6) | 0.5.6 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [backtrace](https://crates.io/crates/backtrace/0.3.76) | 0.3.76 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [base64](https://crates.io/crates/base64/0.22.1) | 0.22.1 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [bitflags](https://crates.io/crates/bitflags/2.13.1) | 2.13.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [block-buffer](https://crates.io/crates/block-buffer/0.10.4) | 0.10.4 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [bumpalo](https://crates.io/crates/bumpalo/3.20.3) | 3.20.3 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [byteorder](https://crates.io/crates/byteorder/1.5.0) | 1.5.0 | 间接/构建/目标平台 | `Unlicense OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [bytes](https://crates.io/crates/bytes/1.10.1) | 1.10.1 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [cc](https://crates.io/crates/cc/1.4.3) | 1.4.3 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [cfg-if](https://crates.io/crates/cfg-if/1.0.4) | 1.0.4 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [cfg_aliases](https://crates.io/crates/cfg_aliases/0.2.2) | 0.2.2 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [cpufeatures](https://crates.io/crates/cpufeatures/0.2.17) | 0.2.17 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [crc32fast](https://crates.io/crates/crc32fast/1.5.0) | 1.5.0 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [crypto-common](https://crates.io/crates/crypto-common/0.1.7) | 0.1.7 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [digest](https://crates.io/crates/digest/0.10.7) | 0.10.7 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [displaydoc](https://crates.io/crates/displaydoc/0.2.7) | 0.2.7 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [encoding_rs](https://crates.io/crates/encoding_rs/0.8.35) | 0.8.35 | 间接/构建/目标平台 | `(Apache-2.0 OR MIT) AND BSD-3-Clause` | 组合许可，保留全部组成义务 |
| [equivalent](https://crates.io/crates/equivalent/1.0.2) | 1.0.2 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [errno](https://crates.io/crates/errno/0.3.14) | 0.3.14 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [fallible-iterator](https://crates.io/crates/fallible-iterator/0.2.0) | 0.2.0 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [fallible-iterator](https://crates.io/crates/fallible-iterator/0.3.0) | 0.3.0 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [fallible-streaming-iterator](https://crates.io/crates/fallible-streaming-iterator/0.1.9) | 0.1.9 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [find-msvc-tools](https://crates.io/crates/find-msvc-tools/0.1.11) | 0.1.11 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [form_urlencoded](https://crates.io/crates/form_urlencoded/1.2.2) | 1.2.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-channel](https://crates.io/crates/futures-channel/0.3.34) | 0.3.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-core](https://crates.io/crates/futures-core/0.3.34) | 0.3.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-io](https://crates.io/crates/futures-io/0.3.34) | 0.3.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-macro](https://crates.io/crates/futures-macro/0.3.31) | 0.3.31 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-sink](https://crates.io/crates/futures-sink/0.3.34) | 0.3.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-task](https://crates.io/crates/futures-task/0.3.34) | 0.3.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [futures-util](https://crates.io/crates/futures-util/0.3.31) | 0.3.31 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [generic-array](https://crates.io/crates/generic-array/0.14.7) | 0.14.7 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [getrandom](https://crates.io/crates/getrandom/0.2.17) | 0.2.17 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [getrandom](https://crates.io/crates/getrandom/0.3.4) | 0.3.4 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [gimli](https://crates.io/crates/gimli/0.32.3) | 0.32.3 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [hashbrown](https://crates.io/crates/hashbrown/0.14.5) | 0.14.5 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [hashbrown](https://crates.io/crates/hashbrown/0.16.1) | 0.16.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [hashlink](https://crates.io/crates/hashlink/0.9.1) | 0.9.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [hex](https://crates.io/crates/hex/0.4.3) | 0.4.3 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [hmac](https://crates.io/crates/hmac/0.12.1) | 0.12.1 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [http](https://crates.io/crates/http/1.5.0) | 1.5.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [http-body](https://crates.io/crates/http-body/1.1.0) | 1.1.0 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [http-body-util](https://crates.io/crates/http-body-util/0.1.5) | 0.1.5 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [httparse](https://crates.io/crates/httparse/1.10.1) | 1.10.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [httpdate](https://crates.io/crates/httpdate/1.0.3) | 1.0.3 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [hyper](https://crates.io/crates/hyper/1.11.0) | 1.11.0 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [hyper-rustls](https://crates.io/crates/hyper-rustls/0.27.7) | 0.27.7 | 直接 | `Apache-2.0 OR ISC OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [hyper-util](https://crates.io/crates/hyper-util/0.1.20) | 0.1.20 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [icu_collections](https://crates.io/crates/icu_collections/2.1.1) | 2.1.1 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [icu_locale_core](https://crates.io/crates/icu_locale_core/2.1.1) | 2.1.1 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [icu_normalizer](https://crates.io/crates/icu_normalizer/2.1.1) | 2.1.1 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [icu_normalizer_data](https://crates.io/crates/icu_normalizer_data/2.1.1) | 2.1.1 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [icu_properties](https://crates.io/crates/icu_properties/2.1.2) | 2.1.2 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [icu_properties_data](https://crates.io/crates/icu_properties_data/2.1.2) | 2.1.2 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [icu_provider](https://crates.io/crates/icu_provider/2.1.1) | 2.1.1 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [idna](https://crates.io/crates/idna/1.1.0) | 1.1.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [idna_adapter](https://crates.io/crates/idna_adapter/1.2.1) | 1.2.1 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [indexmap](https://crates.io/crates/indexmap/2.13.1) | 2.13.1 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [io-uring](https://crates.io/crates/io-uring/0.7.14) | 0.7.14 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [ipnet](https://crates.io/crates/ipnet/2.12.1) | 2.12.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [itoa](https://crates.io/crates/itoa/1.0.18) | 1.0.18 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [jobserver](https://crates.io/crates/jobserver/0.1.34) | 0.1.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [js-sys](https://crates.io/crates/js-sys/0.3.104) | 0.3.104 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [libc](https://crates.io/crates/libc/0.2.189) | 0.2.189 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [libredox](https://crates.io/crates/libredox/0.1.20) | 0.1.20 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [libsqlite3-sys](https://crates.io/crates/libsqlite3-sys/0.30.1) | 0.30.1 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [litemap](https://crates.io/crates/litemap/0.8.3) | 0.8.3 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [lock_api](https://crates.io/crates/lock_api/0.4.14) | 0.4.14 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [log](https://crates.io/crates/log/0.4.33) | 0.4.33 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [lru-slab](https://crates.io/crates/lru-slab/0.1.2) | 0.1.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0 OR Zlib` | 多许可可选；发布时记录选择并保留通知 |
| [matchit](https://crates.io/crates/matchit/0.8.4) | 0.8.4 | 间接/构建/目标平台 | `MIT AND BSD-3-Clause` | 组合许可，保留全部组成义务 |
| [md-5](https://crates.io/crates/md-5/0.10.6) | 0.10.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [memchr](https://crates.io/crates/memchr/2.8.3) | 2.8.3 | 直接 | `Unlicense OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [mime](https://crates.io/crates/mime/0.3.17) | 0.3.17 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [mime_guess](https://crates.io/crates/mime_guess/2.0.5) | 2.0.5 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [miniz_oxide](https://crates.io/crates/miniz_oxide/0.8.9) | 0.8.9 | 间接/构建/目标平台 | `MIT OR Zlib OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [mio](https://crates.io/crates/mio/1.2.2) | 1.2.2 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [multer](https://crates.io/crates/multer/3.1.0) | 3.1.0 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [object](https://crates.io/crates/object/0.37.3) | 0.37.3 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [once_cell](https://crates.io/crates/once_cell/1.21.4) | 1.21.4 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [parking_lot](https://crates.io/crates/parking_lot/0.12.5) | 0.12.5 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [parking_lot_core](https://crates.io/crates/parking_lot_core/0.9.12) | 0.9.12 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [percent-encoding](https://crates.io/crates/percent-encoding/2.3.2) | 2.3.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [phf](https://crates.io/crates/phf/0.11.3) | 0.11.3 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [phf_shared](https://crates.io/crates/phf_shared/0.11.3) | 0.11.3 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [pin-project-lite](https://crates.io/crates/pin-project-lite/0.2.17) | 0.2.17 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [pin-utils](https://crates.io/crates/pin-utils/0.1.0) | 0.1.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [pkg-config](https://crates.io/crates/pkg-config/0.3.34) | 0.3.34 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [plain](https://crates.io/crates/plain/0.2.3) | 0.2.3 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [postgres-protocol](https://crates.io/crates/postgres-protocol/0.6.10) | 0.6.10 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [postgres-types](https://crates.io/crates/postgres-types/0.2.12) | 0.2.12 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [potential_utf](https://crates.io/crates/potential_utf/0.1.6) | 0.1.6 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [ppv-lite86](https://crates.io/crates/ppv-lite86/0.2.21) | 0.2.21 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [proc-macro2](https://crates.io/crates/proc-macro2/1.0.107) | 1.0.107 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [quinn](https://crates.io/crates/quinn/0.11.9) | 0.11.9 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [quinn-proto](https://crates.io/crates/quinn-proto/0.11.14) | 0.11.14 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [quinn-udp](https://crates.io/crates/quinn-udp/0.5.14) | 0.5.14 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [quote](https://crates.io/crates/quote/1.0.47) | 1.0.47 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [r-efi](https://crates.io/crates/r-efi/5.3.0) | 5.3.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | 有 MIT/Apache-2.0 可选分支；不选择 LGPL |
| [rand](https://crates.io/crates/rand/0.9.5) | 0.9.5 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [rand_chacha](https://crates.io/crates/rand_chacha/0.9.0) | 0.9.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [rand_core](https://crates.io/crates/rand_core/0.9.5) | 0.9.5 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [redox_syscall](https://crates.io/crates/redox_syscall/0.5.18) | 0.5.18 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [redox_syscall](https://crates.io/crates/redox_syscall/0.9.2) | 0.9.2 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [reqwest](https://crates.io/crates/reqwest/0.12.22) | 0.12.22 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [ring](https://crates.io/crates/ring/0.17.14) | 0.17.14 | 直接 | `Apache-2.0 AND ISC` | 组合许可，保留全部组成义务 |
| [rusqlite](https://crates.io/crates/rusqlite/0.32.1) | 0.32.1 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [rustc-demangle](https://crates.io/crates/rustc-demangle/0.1.28) | 0.1.28 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [rustc-hash](https://crates.io/crates/rustc-hash/2.1.3) | 2.1.3 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [rustls](https://crates.io/crates/rustls/0.23.43) | 0.23.43 | 间接/构建/目标平台 | `Apache-2.0 OR ISC OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [rustls-pki-types](https://crates.io/crates/rustls-pki-types/1.15.1) | 1.15.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [rustls-webpki](https://crates.io/crates/rustls-webpki/0.103.14) | 0.103.14 | 间接/构建/目标平台 | `ISC` | 按所声明许可保留版权和许可要求 |
| [rustversion](https://crates.io/crates/rustversion/1.0.23) | 1.0.23 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [ryu](https://crates.io/crates/ryu/1.0.23) | 1.0.23 | 间接/构建/目标平台 | `Apache-2.0 OR BSL-1.0` | 多许可可选；发布时记录选择并保留通知 |
| [scopeguard](https://crates.io/crates/scopeguard/1.2.0) | 1.2.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [serde](https://crates.io/crates/serde/1.0.219) | 1.0.219 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [serde_derive](https://crates.io/crates/serde_derive/1.0.219) | 1.0.219 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [serde_json](https://crates.io/crates/serde_json/1.0.142) | 1.0.142 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [serde_path_to_error](https://crates.io/crates/serde_path_to_error/0.1.17) | 0.1.17 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [serde_urlencoded](https://crates.io/crates/serde_urlencoded/0.7.1) | 0.7.1 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [serde_yaml](https://crates.io/crates/serde_yaml/0.9.34-deprecated) | 0.9.34-deprecated | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [sha2](https://crates.io/crates/sha2/0.10.9) | 0.10.9 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [shlex](https://crates.io/crates/shlex/2.0.1) | 2.0.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [signal-hook-registry](https://crates.io/crates/signal-hook-registry/1.4.8) | 1.4.8 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [siphasher](https://crates.io/crates/siphasher/1.0.3) | 1.0.3 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [slab](https://crates.io/crates/slab/0.4.12) | 0.4.12 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [smallvec](https://crates.io/crates/smallvec/1.15.2) | 1.15.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [socket2](https://crates.io/crates/socket2/0.5.10) | 0.5.10 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [socket2](https://crates.io/crates/socket2/0.6.5) | 0.6.5 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [spin](https://crates.io/crates/spin/0.9.9) | 0.9.9 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [stable_deref_trait](https://crates.io/crates/stable_deref_trait/1.2.1) | 1.2.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [stringprep](https://crates.io/crates/stringprep/0.1.5) | 0.1.5 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [subtle](https://crates.io/crates/subtle/2.6.1) | 2.6.1 | 间接/构建/目标平台 | `BSD-3-Clause` | 按所声明许可保留版权和许可要求 |
| [syn](https://crates.io/crates/syn/2.0.119) | 2.0.119 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [syn](https://crates.io/crates/syn/3.0.3) | 3.0.3 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [sync_wrapper](https://crates.io/crates/sync_wrapper/1.0.2) | 1.0.2 | 间接/构建/目标平台 | `Apache-2.0` | 按所声明许可保留版权和许可要求 |
| [synstructure](https://crates.io/crates/synstructure/0.13.2) | 0.13.2 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [thiserror](https://crates.io/crates/thiserror/2.0.20) | 2.0.20 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [thiserror-impl](https://crates.io/crates/thiserror-impl/2.0.20) | 2.0.20 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [tinystr](https://crates.io/crates/tinystr/0.8.4) | 0.8.4 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [tinyvec](https://crates.io/crates/tinyvec/1.12.0) | 1.12.0 | 间接/构建/目标平台 | `Zlib OR Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [tinyvec_macros](https://crates.io/crates/tinyvec_macros/0.1.1) | 0.1.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0 OR Zlib` | 多许可可选；发布时记录选择并保留通知 |
| [tokio](https://crates.io/crates/tokio/1.47.1) | 1.47.1 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tokio-macros](https://crates.io/crates/tokio-macros/2.5.0) | 2.5.0 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tokio-postgres](https://crates.io/crates/tokio-postgres/0.7.13) | 0.7.13 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [tokio-rustls](https://crates.io/crates/tokio-rustls/0.26.4) | 0.26.4 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [tokio-stream](https://crates.io/crates/tokio-stream/0.1.17) | 0.1.17 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tokio-util](https://crates.io/crates/tokio-util/0.7.16) | 0.7.16 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tower](https://crates.io/crates/tower/0.5.3) | 0.5.3 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tower-http](https://crates.io/crates/tower-http/0.6.11) | 0.6.11 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tower-layer](https://crates.io/crates/tower-layer/0.3.3) | 0.3.3 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tower-service](https://crates.io/crates/tower-service/0.3.3) | 0.3.3 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tracing](https://crates.io/crates/tracing/0.1.44) | 0.1.44 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [tracing-core](https://crates.io/crates/tracing-core/0.1.36) | 0.1.36 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [try-lock](https://crates.io/crates/try-lock/0.2.5) | 0.2.5 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [typenum](https://crates.io/crates/typenum/1.20.1) | 1.20.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [unicase](https://crates.io/crates/unicase/2.9.0) | 2.9.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [unicode-bidi](https://crates.io/crates/unicode-bidi/0.3.18) | 0.3.18 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [unicode-ident](https://crates.io/crates/unicode-ident/1.0.24) | 1.0.24 | 间接/构建/目标平台 | `(MIT OR Apache-2.0) AND Unicode-3.0` | 组合许可，保留全部组成义务 |
| [unicode-normalization](https://crates.io/crates/unicode-normalization/0.1.25) | 0.1.25 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [unicode-properties](https://crates.io/crates/unicode-properties/0.1.4) | 0.1.4 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [unsafe-libyaml](https://crates.io/crates/unsafe-libyaml/0.2.11) | 0.2.11 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [untrusted](https://crates.io/crates/untrusted/0.9.0) | 0.9.0 | 间接/构建/目标平台 | `ISC` | 按所声明许可保留版权和许可要求 |
| [url](https://crates.io/crates/url/2.5.4) | 2.5.4 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [utf8_iter](https://crates.io/crates/utf8_iter/1.0.4) | 1.0.4 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [vcpkg](https://crates.io/crates/vcpkg/0.2.15) | 0.2.15 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [version_check](https://crates.io/crates/version_check/0.9.5) | 0.9.5 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |
| [want](https://crates.io/crates/want/0.3.1) | 0.3.1 | 间接/构建/目标平台 | `MIT` | 按所声明许可保留版权和许可要求 |
| [wasi](https://crates.io/crates/wasi/0.11.1+wasi-snapshot-preview1) | 0.11.1+wasi-snapshot-preview1 | 间接/构建/目标平台 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [wasip2](https://crates.io/crates/wasip2/1.0.1+wasi-0.2.4) | 1.0.1+wasi-0.2.4 | 间接/构建/目标平台 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [wasite](https://crates.io/crates/wasite/0.1.0) | 0.1.0 | 间接/构建/目标平台 | `Apache-2.0 OR BSL-1.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [wasm-bindgen](https://crates.io/crates/wasm-bindgen/0.2.127) | 0.2.127 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [wasm-bindgen-futures](https://crates.io/crates/wasm-bindgen-futures/0.4.77) | 0.4.77 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [wasm-bindgen-macro](https://crates.io/crates/wasm-bindgen-macro/0.2.127) | 0.2.127 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [wasm-bindgen-macro-support](https://crates.io/crates/wasm-bindgen-macro-support/0.2.127) | 0.2.127 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [wasm-bindgen-shared](https://crates.io/crates/wasm-bindgen-shared/0.2.127) | 0.2.127 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [wasm-streams](https://crates.io/crates/wasm-streams/0.4.2) | 0.4.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [web-sys](https://crates.io/crates/web-sys/0.3.104) | 0.3.104 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [web-time](https://crates.io/crates/web-time/1.1.0) | 1.1.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [webpki-roots](https://crates.io/crates/webpki-roots/1.0.9) | 1.0.9 | 间接/构建/目标平台 | `CDLA-Permissive-2.0` | 数据许可；保留其分发要求 |
| [whoami](https://crates.io/crates/whoami/1.6.1) | 1.6.1 | 间接/构建/目标平台 | `Apache-2.0 OR BSL-1.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [windows-link](https://crates.io/crates/windows-link/0.2.1) | 0.2.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows-sys](https://crates.io/crates/windows-sys/0.52.0) | 0.52.0 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows-sys](https://crates.io/crates/windows-sys/0.59.0) | 0.59.0 | 直接 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows-sys](https://crates.io/crates/windows-sys/0.60.2) | 0.60.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows-sys](https://crates.io/crates/windows-sys/0.61.2) | 0.61.2 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows-targets](https://crates.io/crates/windows-targets/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows-targets](https://crates.io/crates/windows-targets/0.53.5) | 0.53.5 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_aarch64_gnullvm](https://crates.io/crates/windows_aarch64_gnullvm/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_aarch64_gnullvm](https://crates.io/crates/windows_aarch64_gnullvm/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_aarch64_msvc](https://crates.io/crates/windows_aarch64_msvc/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_aarch64_msvc](https://crates.io/crates/windows_aarch64_msvc/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_i686_gnu](https://crates.io/crates/windows_i686_gnu/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_i686_gnu](https://crates.io/crates/windows_i686_gnu/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_i686_gnullvm](https://crates.io/crates/windows_i686_gnullvm/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_i686_gnullvm](https://crates.io/crates/windows_i686_gnullvm/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_i686_msvc](https://crates.io/crates/windows_i686_msvc/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_i686_msvc](https://crates.io/crates/windows_i686_msvc/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_x86_64_gnu](https://crates.io/crates/windows_x86_64_gnu/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_x86_64_gnu](https://crates.io/crates/windows_x86_64_gnu/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_x86_64_gnullvm](https://crates.io/crates/windows_x86_64_gnullvm/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_x86_64_gnullvm](https://crates.io/crates/windows_x86_64_gnullvm/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.52.6) | 0.52.6 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [windows_x86_64_msvc](https://crates.io/crates/windows_x86_64_msvc/0.53.1) | 0.53.1 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [wit-bindgen](https://crates.io/crates/wit-bindgen/0.46.0) | 0.46.0 | 间接/构建/目标平台 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [writeable](https://crates.io/crates/writeable/0.6.4) | 0.6.4 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [yoke](https://crates.io/crates/yoke/0.8.3) | 0.8.3 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [yoke-derive](https://crates.io/crates/yoke-derive/0.8.2) | 0.8.2 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [zerocopy](https://crates.io/crates/zerocopy/0.8.56) | 0.8.56 | 间接/构建/目标平台 | `BSD-2-Clause OR Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [zerocopy-derive](https://crates.io/crates/zerocopy-derive/0.8.56) | 0.8.56 | 间接/构建/目标平台 | `BSD-2-Clause OR Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [zerofrom](https://crates.io/crates/zerofrom/0.1.8) | 0.1.8 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [zerofrom-derive](https://crates.io/crates/zerofrom-derive/0.1.7) | 0.1.7 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [zeroize](https://crates.io/crates/zeroize/1.8.2) | 1.8.2 | 间接/构建/目标平台 | `Apache-2.0 OR MIT` | 多许可可选；发布时记录选择并保留通知 |
| [zerotrie](https://crates.io/crates/zerotrie/0.2.5) | 0.2.5 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [zerovec](https://crates.io/crates/zerovec/0.11.7) | 0.11.7 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [zerovec-derive](https://crates.io/crates/zerovec-derive/0.11.4) | 0.11.4 | 间接/构建/目标平台 | `Unicode-3.0` | 保留 Unicode 数据/版权许可要求 |
| [zstd](https://crates.io/crates/zstd/0.13.3) | 0.13.3 | 直接 | `MIT` | 按所声明许可保留版权和许可要求 |
| [zstd-safe](https://crates.io/crates/zstd-safe/7.2.4) | 7.2.4 | 间接/构建/目标平台 | `MIT OR Apache-2.0` | 多许可可选；发布时记录选择并保留通知 |
| [zstd-sys](https://crates.io/crates/zstd-sys/2.0.16+zstd.1.5.7) | 2.0.16+zstd.1.5.7 | 间接/构建/目标平台 | `MIT/Apache-2.0` | 历史双许可写法；发布时确认许可原文 |

## 本轮不包含的结论

没有声称所有可选功能、所有目标平台、vendored 文件或未来新依赖已经获准分发；也没有因服务进程通信就认定可以自由复制 GPL 插件代码。新增依赖须更新此表并核对其实际许可。
