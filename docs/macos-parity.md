# macOS 适配验收记录

目标：完整复刻 Windows 基线 `03454a8`，不是核心功能演示。

当前状态：实施中。尚无完整 Mac 安装包、真实账号验收或正式分发签名。

## 已观测证据（2026-11-15）

- 本机 ARM64 / macOS 27.2.1 / 官方 app 26.1111.21308。
- 临时 HOME、CODEX_HOME、独立 user-data-dir 下启动自己的实例：loopback CDP 可用，真实页面 `app://-/index.html` 可访问。
- 该临时页面最小 DOM 注入并撤销成功；原有客户端 PID 在验证前后保持运行。这不是完整插件或登录会话验收。
- 原始 TypeScript typecheck 通过；19 项插件发布合同测试通过。
- 原始 UI build 在首次运行因法律声明的 CRLF 指纹不匹配失败；显式 checkout 行尾规则后，原锁定插件字节验证及 UI build 通过。
- 上游架构检查原有两个失败：ascii-tiles/spectral-drape 的 settings 直接引用 parameter-panel。共享面板移至 components 后，57 项核心合同测试通过，类型检查和 UI 构建再验证通过。
- 本地探测/构建日志在 `artifacts/macos-port/`，不含真实账号凭证。

## 系统功能

| 项目 | 状态 | 说明 |
| --- | --- | --- |
| 真实隔离宿主 CDP / 最小 DOM 操作 | 基础验证通过 | 完整 UI/bridge 集成待验证 |
| 应用发现/CLI/资源定位 | 实现中 | 非标准 app 名称与签名 |
| 进程/socket 校验与 attach 生命周期 | 实现中 | 原生身份及 fail-closed |
| 文件服务/路径/存储/Finder | 实现中 | 大小写/Unicode/链接边界 |
| 账号/密钥库/系统菜单 | 未完成 | 无真实账号验证 |
| 原生窗口透明 | 未验证 | 禁止 CSS 代替原生结果 |
| 旧版 PPT 原生导出 | 未验证 | 需 Mac Office 合同与实测 |
| 安装/更新/登录启动/卸载 | 未完成 | Mac 生命周期尚未集成 |
| 正式签名/公证/发布 | 未完成 | 需要独立发布通道与签名条件 |
| Windows 回归 | 未验证 | 本机不是 Windows runner |

## 插件逐项记录

“未验证”表示尚未完成 Mac 上真实启用、参数保存、停用和恢复验证；定义存在或构建成功不等于通过。

| 插件 | 版本 | 类别 | Mac 实测 |
| --- | --- | --- | --- |
| glow-horizon | 1.0.1 | appearance | 未验证 |
| black-hole | 1.0.6 | appearance | 未验证 |
| heavenly-cloud | 1.0.1 | appearance | 未验证 |
| aurora-ionosphere | 1.0.1 | appearance | 未验证 |
| milky-way | 1.0.1 | appearance | 未验证 |
| mountain | 1.0.1 | appearance | 未验证 |
| cloud-train | 1.0.1 | appearance | 未验证 |
| blinking-squares | 1.0.1 | appearance | 未验证 |
| particle-image | 1.0.5 | appearance | 未验证 |
| pixel-sculpt | 1.0.13 | appearance | 未验证 |
| custom-background | 1.0.13 | appearance | 未验证 |
| surface-opacity | 1.0.14 | appearance | 未验证 |
| transparent-background | 1.0.0 | appearance | 未验证 |
| git-history | 1.0.0 | developer-tools | 未验证 |
| codex-startup-transition | 1.1.33 | appearance | 未验证 |
| markdown-preview | 1.0.0 | file-preview | 未验证 |
| csv-preview | 1.0.1 | file-preview | 未验证 |
| diagram-preview | 1.0.1 | file-preview | 未验证 |
| image-preview | 1.0.0 | file-preview | 未验证 |
| video-preview | 1.0.0 | file-preview | 未验证 |
| pdf-preview | 1.0.1 | file-preview | 未验证 |
| audio-preview | 1.0.0 | file-preview | 未验证 |
| office-preview | 1.0.0 | file-preview | 未验证 |
| notebook-preview | 1.0.1 | file-preview | 未验证 |
| model-preview | 1.0.0 | file-preview | 未验证 |
| ascii-tiles | 1.0.8 | appearance | 未验证 |
| shallow-ocean | 1.0.17 | appearance | 未验证 |
| multi-account | 1.0.110 | developer-tools | 未验证 |
| spectral-drape | 1.0.2 | appearance | 未验证 |
