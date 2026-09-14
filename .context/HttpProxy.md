Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！
Proxy 和 Provider 是稳定的！

强制命令：本项目是 CC-Switch，只增加 Kilo 接入，必须直接复用项目已有实现。

1. Kilo 需要 HTTP 请求，就使用项目原有请求库和对应的发送入口；需要 Proxy，就使用项目原有代理实现。
2. 只增加 Kilo 配置读取和调用现有能力所必需的接入代码。禁止新增客户端工厂、请求封装、代理机制或另一套网络路径。
3. 禁止修改原有共享实现、影响其他 AppType，禁止更换原有请求库、发送路径或 TLS 策略，禁止无谓重构和扩展。
4. 禁止以“复用检测客户端”“统一检测与转发”“方便后续扩展”为理由改变原有实现。
5. 如认为现有能力无法满足要求，先指出具体代码位置和缺口，停止相关实现并说明；不得自行新增替代方案。
6. 本命令优先于下文方案。下文若要求新增独立客户端或切换发送路径，视为待纠正的方案错误，不得照做。

## Proxy 实现评估补充（2026-11-05）

本节记录对用户提供的 macOS `scutil --proxy` / reqwest 实现的评估，以及当前仓库代码核对结果。以下借鉴点不是实施任务，不授权修改共享代理实现，也不改变 Kilo 的显式代理约定。

### 当前实现与依赖事实

- `src-tauri/Cargo.toml` 的 reqwest 0.12 未关闭默认 features；`src-tauri/Cargo.lock` 当前锁定 `reqwest 0.12.28`、`hyper-util 0.1.20`。reqwest 默认启用 `system-proxy`，底层通过 SystemConfiguration API 读取 macOS 系统 HTTP/HTTPS 代理。因此，`proxy/http_client.rs` 中“系统代理（环境变量）”的注释并未完整描述实际行为，不应据此判断项目缺少 macOS 系统代理读取能力。
- 底层先读取环境变量，再分别补齐尚未配置的系统 HTTP/HTTPS 代理；其 macOS 读取实现未处理系统 SOCKS、`ExceptionsList`、`ExcludeSimpleHostnames` 或 PAC。项目启用 `socks` feature 只说明 reqwest 支持显式 SOCKS 代理，不代表会自动读取 macOS 系统 SOCKS 配置。
- 全局 reqwest 客户端的 `build_client_from` 在未填写代理时通常保留自动代理探测；只有现有防自环检查命中时调用 `.no_proxy()`。界面“留空表示直连”的文案与该路径实际行为不一致，此结论不应套用到 Kilo。
- `proxy/http_client.rs` 的 `get_for_url` 为本机目标选择直连客户端；这不等于完整支持 macOS 系统绕过列表。`system_proxy_points_to_loopback` 仅检查环境变量，且只有代理主机为 loopback、端口为本应用监听端口时才禁用自动代理；它未检查 macOS 系统配置来源，也不会排除其他端口的本地代理工具。
- 上述自动代理行为属于全局 reqwest 客户端，不代表所有请求入口。当前 `proxy/kilo.rs` 的 `check_provider` 和实际转发都已读取 `extract_proxy`，将 Provider proxy 显式传入现有 `hyper_client::send_request`。Kilo 不应为了跟随系统代理新增客户端或改变发送路径。
- 下文仍保留早期阶段方案，其中“实际转发尚未接入”、Kilo 专用 `build_http_client` 工厂及切换为 `ProxyResponse::Reqwest` 的描述已不符合当前代码；不得据此重新实现。以当前代码及本文顶部的复用约束为准。

### 参考实现可借鉴的内容

- 按目标 HTTP/HTTPS 协议选择对应代理，缺失时使用已启用的 SOCKS 代理；HTTPS 目标使用 `http://` 代理地址本身合理，目标协议与代理连接协议是两回事。
- 读取系统绕过列表及简单主机名排除规则，并考虑与环境 `NO_PROXY` 合并；系统 SOCKS 使用 `socks5h` 时由代理解析目标域名。
- 将配置读取、解析和代理选择拆开，环境变量读取可注入，便于独立验证。此处仅记录设计优点，不增加本项目测试或测试矩阵。

### 不能直接照搬的部分

- 参考实现发现任意一个非空代理环境变量就完全跳过系统配置，与当前依赖按协议补齐的策略不同。例如只设置 `HTTP_PROXY` 时，HTTPS 是否继续使用系统代理会有差异，不能无意改变既有语义。
- 参考实现没有本应用代理端口的防自环保护。若未来单独获准补强系统代理，需同时考虑系统配置来源，不能仅复制解析代码。
- 将 `*.example.com` 转成 `.example.com` 可能扩大匹配范围，其他通配符也不能假定与 reqwest `NoProxy` 完全等价。参考测试只检查绕过条目转换和简单主机名选择，未验证绕过列表的实际请求行为。
- 参考实现未处理 PAC，也未监听系统代理变更；`scutil` 采用同步子进程调用，其耗时不受 HTTP 请求的 15 秒超时控制。不能在每次请求中直接读取，也不能将其视为完整的系统代理支持。
- 15 秒总超时针对小型 registry 响应，不适用于直接替换现有流式转发配置。读取系统配置如确有独立需求，可优先评估现有依赖使用的原生 SystemConfiguration API，避免依赖命令输出格式；本轮不新增实现或依赖。

结论：可吸收系统 SOCKS、绕过规则和解析结构的思路；macOS HTTP/HTTPS 系统代理检测已有依赖支持，无需重复建设。共享能力缺口仅在此记录，Kilo 继续遵循“有值使用指定代理、无值显式直连”的现有接入约定。

# Kilo Provider 级 HTTP/HTTPS Proxy 实施方案

## 状态与范围

- 2026-10-16：第一阶段（表单、JSON 与数据库保存）和第二阶段（连通性检测）已完成；下一步接入实际请求转发。
- Proxy 和 Provider 都非常稳定，全部从简设计，不新增测试、测试矩阵或极端情况处理；不主动调整 TLS 实现、证书策略或重试机制。
- 只在 Kilo 内增加本功能所必需的能力，业务逻辑集中在 `proxy/kilo.rs`；不修改原有共享实现或其他 AppType。
- 本文已按最新约定调整：不增加独立的后端代理解析/校验步骤，也不新增后端格式校验器；读取配置属于检测和转发各自的接入工作。
- 仅处理 AppType=`Kilo` 的 Provider 代理；实际转发范围为 Routing 模式。
- 配置保存在现有 `settingsConfig.options.proxy` 中，不增加数据库结构、迁移或独立配置文件。
- 不扩展其他 AppType，不修改共享 Hyper 的选路、全局代理设置或进程环境。
- Provider proxy 已用于连通性检测；当前实际转发仍读取全局代理，尚未使用 Provider proxy。

## 配置与执行约定

`proxy` 是字符串，不是对象，没有 mode 字段：

```json
{
  "options": {
    "baseURL": "https://provider.example.com/v1",
    "apiKey": "...",
    "proxy": "http://127.0.0.1:7890"
  }
}
```

1. `options.proxy` 有值就使用该代理，没值就直连，不存在继承模式。
2. 读取时使用字符串值并 trim；缺失、空字符串、纯空白按无值处理。非字符串不新增类型校验，使用 `as_str()` 读取不到时按无值处理。
3. 有值直接交给网络库构建代理。不新增 scheme、host、端口、userinfo、路径、query、fragment 等后端业务校验，也不接入完整 Kilo Provider parser。
4. 面向用户的功能范围为 HTTP/HTTPS Proxy；地址不可用时由网络库返回构造或连接错误，不通过自定义校验器预先限制。
5. Kilo 显式客户端忽略全局、系统和 HTTP_PROXY/HTTPS_PROXY/ALL_PROXY/NO_PROXY 等环境代理规则。
6. 指定代理后，本机上游也走该代理；失败直接报错，不回退直连。无值时必须显式直连。
7. 第一阶段已有的前端校验不在本轮变更范围；不因本约定额外新增后端保存/导入校验。

## 实施阶段与顺序

### 第一阶段：表单、JSON 与数据库保存（已完成）

- Kilo 表单已提供单个可选代理输入框。
- 预览和提交输出 trim 后的字符串，清空时写出 `"proxy": ""`。
- 沿用现有完整 JSON 保存和加载流程，无数据库迁移。
- 检测已接入，实际转发尚未接入；当前表单未提供阶段说明提示。

### 第二阶段：接入连通性检测（已完成）

具体方案见 [current-task.md](./current-task.md)。

- 在 `services/stream_check.rs` 的 `check_once` 中为 Kilo 单独选择客户端。
- 在 `proxy/kilo.rs` 提供 Kilo 专用显式 reqwest 客户端工厂 `build_http_client`，先 `.no_proxy()`，有值再加入 `reqwest::Proxy::all(value)`。
- 仅读取传入 Provider 的 `settings_config.options.proxy`；无值直连，有值使用代理，不增加后端格式校验。
- 保持 GET baseURL、收到任意 HTTP 响应即成功、现有超时/重试/状态结果语义。
- 其他 AppType 保持 `get_for_url`，实际模型请求本阶段不改。
- 配置读取与客户端构造集中在 `proxy/kilo.rs`，检测入口仅调用 `reachability_client`；不以补充测试或阶段提示作为本阶段完成条件。

### 第三阶段：接入实际请求转发（下一步）

我们在给 Kilo 增加功能，不要去碰原来的实现，不允许做任何无谓的扩展。

Proxy 和 Provider 都非常稳定，从简实现，不新增测试，不再拆分 raw 路径、原始请求头大小写或 IPv6 等专项阶段。

- 仅在 `proxy/kilo.rs` 中将实际请求接入 Provider proxy，读取实际选中 Provider 的配置。
- 复用已有 `extract_proxy` 和 `build_http_client`：有值使用该代理，无值显式直连；Kilo 发送处不再读取全局代理。
- 使用该 Kilo 客户端发送请求，包装为现有 `ProxyResponse::Reqwest`，继续复用原有响应处理。
- 等待响应头沿用现有 `timeout_for`，不设置整个流式响应的总超时。
- 构造或发送失败进入现有失败处理，不增加重试、补偿或直连回退。
- 不修改共享 Hyper、全局客户端或其他 AppType，不扩展请求头大小写保留、raw/reqwest 分支选择、TLS 策略、缓存或故障转移。
- 原有 SSE、统计、失败记录和生命周期处理保持不动。

### 第四阶段：简要检查与收尾

- 按稳定配置从简检查正常使用路径与检测/转发的一致性，不新增测试、证书矩阵或跨平台验证项目。
- 沿用现有 TLS、SSE、统计与失败处理，不为本功能重构原有能力。
- 全部接入后如需表单提示，使用最终语义：“留空直连，不使用全局代理；填写后，此供应商的 HTTP/HTTPS 请求均使用该代理。”
- 简要审查 diff，确认改动集中在 Kilo，未引入全局代理继承或失败直连回退。

## 网络客户端约束

- 独立构造 Kilo client，不调用全局 `get/get_for_url/direct_client` 或全局更新接口。
- 显式禁用自动代理探测；指定代理不套用本机地址或环境 NO_PROXY 绕过。
- 第二阶段 TLS 沿用现有 HTTP 客户端方式，保留正常证书和主机名验证，不主动切换 TLS 实现或新增系统证书加载策略。第三阶段复用已有 Kilo 客户端，不扩展 TLS 兼容性工作。
- 保留连接超时、连接池、keepalive 与禁用自动解压的现有参数；不复用含 600 秒总超时的 `base_builder()`。
- 禁用自动重定向；client 构造失败返回错误，禁止回退默认、全局或直连 client。
- 检测超时由单次请求配置控制；为第三阶段保留无流式总超时的工厂能力。
- 不新增用户名/密码表单或独立认证功能；本轮不增加 userinfo 拒绝校验。

## 持久化与兼容性

- 不修改 DAO SQL、schema 或数据库版本；代理随现有 settingsConfig 同步和导出。
- 不写入 Provider.meta、客户端配置、live-state.json 或全局 settings。
- 检测接入后，无 proxy 的旧 Provider 检测将明确直连；实际转发在第三阶段接入后才改变。
- 不自动迁移全局代理，不做设备本地覆盖层。

## 验证原则

Proxy 和 Provider 都非常稳定，全部从简设计，不为本功能新增测试或测试矩阵，也不把缺少额外测试视为进度缺口。已有测试保留，不扩展测试基础设施，不为运行本功能测试修复无关模块。

业务改动只做必要的编译、类型或格式检查；文档修改只检查 diff，不执行业务测试。优先确保 Kilo 新能力接入正确、原有逻辑改动最小。
