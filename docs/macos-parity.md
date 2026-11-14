# macOS 适配验收记录

目标：完整复刻 Windows 基线 `03454a8`，不是核心功能演示。

当前状态：已有 ARM64 本地开发包，仍在综合验收；不具备完整 Windows 功能验收、真实账号验收或正式分发签名。

## 已观测证据（2026-11-15）

- 本机 ARM64 / macOS 27.2.1 / 官方 app 26.1111.21308。
- 临时 HOME、CODEX_HOME、独立 user-data-dir 下启动自己的实例：loopback CDP 可用，真实页面 `app://-/index.html` 可访问。
- 该临时页面最小 DOM 注入并撤销成功；原有客户端 PID 在验证前后保持运行。这不是完整插件或登录会话验收。
- 原始 TypeScript typecheck 通过；19 项插件发布合同测试通过。
- 原始 UI build 在首次运行因法律声明的 CRLF 指纹不匹配失败；显式 checkout 行尾规则后，原锁定插件字节验证及 UI build 通过。
- 上游架构检查原有两个失败：ascii-tiles/spectral-drape 的 settings 直接引用 parameter-panel。共享面板移至 components 后，57 项核心合同测试通过，类型检查和 UI 构建再验证通过。
- 最终 Rust 工作区测试：782通过、0失败、15忽略。包含launcher460、GUI24、workspace-service121；两项原有Windows表面补丁测试和两项Windows快捷方式专用测试按平台门控，不能视为Mac通过。
- 最终UI typecheck/build、架构检查（252文件/678依赖/0循环）、i18n（1247对齐键）、67项核心Node合同和3项Python打包测试通过。
- release核心/GUI生成本地ad-hoc签名app、ZIP、DMG；真实二进制在临时中文安装路径完成安装、错误SHA拒绝、升级、回滚及签名复核。没有执行当前用户的生产卸载或登录项变更。
- 最终只读官方应用扫描覆盖24,315项，complete=true且issues为空，包括ASAR成员和未跟随的内部framework链接。这只证明资源盘点完整，不证明功能兼容。
- 隔离探测脚本的三项真实临时进程测试通过：父进程提前退出、子进程忽略TERM、报告写入失败均不会跳过所属进程组清理；独立审阅发现已关闭。现有官方客户端未被中断。
- 账户 Keychain 用随机临时目录及合成值验证，没有读取真实凭证；登录项/重启对话框尚未用于当前活动客户端。
- 依赖审计发现基线 dompurify critical 和 markdown-it high 通告；锁定发布插件尚未重新发布修复版本，正式发布前必须消除或完成逐项可达性评估，不能把构建通过当作无漏洞。
- 本地探测/构建日志在 `artifacts/macos-port/`，不含真实账号凭证。

## 系统功能

| 项目 | 状态 | 说明 |
| --- | --- | --- |
| 真实隔离宿主 CDP / 最小 DOM 操作 | 基础验证通过 | 原生端口身份通过；隔离未登录页被布局检查拒绝，完整注入未通过 |
| 应用发现/CLI/资源定位 | 已实现、定向验证通过 | 签名/非标准 app 名/嵌入 CLI；真实 Beta 未验 |
| 进程/socket 校验与 attach 生命周期 | 已实现、定向验证通过 | libproc/kqueue、loopback 所有者、重连保留身份；PID 复用与跨用户测试未验 |
| 文件服务/路径/存储/Finder | 已实现、APFS 测试通过 | 默认及大小写敏感卷；路径复制和草稿 resync 审阅问题已修；Finder 视觉待验 |
| 账号/密钥库/系统菜单 | 基础实现及合成测试通过 | 账户166通过9忽略；本地 API 绕过代理、外部 HTTPS 保留代理；无真实账号/加密宿主投影验收 |
| 原生窗口透明 | 未验证 | 禁止 CSS 代替原生结果 |
| 旧版 PPT 原生导出 | 阻塞 | PowerPoint16.114.4合成PPT/PPTX打开-9074，宏禁用-10006；未导出PNG，未推断原因 |
| 安装/更新/登录启动/卸载 | 本地实现及7项合成安装测试通过 | 包含保留数据卸载、明确清除确认；真实GUI退出/登录项待验；在线Mac feed待配置 |
| 正式签名/公证/发布 | 未完成 | 需要独立发布通道与签名条件 |
| Windows 回归 | 未验证 | 本机不是 Windows runner |

## 插件逐项记录

本机 Chrome 157 无头夹具。29 个发布包及其资源均按锁重算 SHA-256；市场夹具通过 63 项断言。锁定包表示实际发布字节进入测试；源 Worker 使用仓库源文件；宿主与原生服务按各夹具模拟。真实宿主未完成任何一项完整验收。

| 插件 | 版本 | 类别 | 离线实测与缺口 |
| --- | --- | --- | --- |
| glow-horizon | 1.0.1 | appearance | 锁定包浏览器热启停、离线复用、草稿保留；待逐项参数与冷重启 |
| black-hole | 1.0.6 | appearance | 源 Worker 首帧/回收、WebGL 像素淡入；待锁定包完整参数/恢复 |
| heavenly-cloud | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 |
| aurora-ionosphere | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 |
| milky-way | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 |
| mountain | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 |
| cloud-train | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 |
| blinking-squares | 1.0.1 | appearance | 源 Worker 绘制与回收；锁定包启停/深浅/系统主题恢复 |
| particle-image | 1.0.5 | appearance | 源 Worker 绘制与回收；锁定包软件 WebGL 旋转/设置测试 |
| pixel-sculpt | 1.0.13 | appearance | 源 Worker 数据与绘制回收；真实 controller 与替代 GPU 生命周期；待锁定包完整编辑 |
| custom-background | 1.0.13 | appearance | 锁定包主工作区/预览窗口样式与清理；共享 CSS 合同 |
| surface-opacity | 1.0.14 | appearance | 锁定包完整离线夹具通过（32初始/6冷登录/6视频就绪及拖动、帧解码、窄屏等）；预览标签保持透明，原生菜单与预览背景独立 |
| transparent-background | 1.0.0 | appearance | 锁定包 CSS 启停与模拟原生 RPC 通过；G3 原生透明阻塞 |
| git-history | 1.0.0 | developer-tools | 锁定包面板、合成提交、关面板/晚回复回收；待实际宿主文件和仓库联动 |
| codex-startup-transition | 1.1.33 | appearance | 锁定包离线早启动、背景/视频、缺包/禁用跳过、设置及退出 |
| markdown-preview | 1.0.0 | file-preview | 锁定包 Markdown 渲染、禁用/缓存重启 |
| csv-preview | 1.0.1 | file-preview | 锁定包 CSV 表格、坏引号警告、禁用/缓存重启 |
| diagram-preview | 1.0.1 | file-preview | 锁定包 PlantUML、非法 Draw.io 错误、取消、禁用/缓存重启 |
| image-preview | 1.0.0 | file-preview | 锁定包 PNG 解码、禁用/缓存重启 |
| video-preview | 1.0.0 | file-preview | 锁定包 WebM 元数据、禁用/缓存重启 |
| pdf-preview | 1.0.1 | file-preview | 锁定包 PDF 画布、错误/取消、禁用/缓存重启 |
| audio-preview | 1.0.0 | file-preview | 锁定包 WAV 元数据、禁用/缓存重启 |
| office-preview | 1.0.0 | file-preview | 锁定包 DOCX、Worker/WASM 资源、错误/取消、禁用/缓存重启；旧版 PPT 原生导出阻塞 |
| notebook-preview | 1.0.1 | file-preview | 锁定包 Notebook 单元格、坏 JSON/取消、禁用/缓存重启 |
| model-preview | 1.0.0 | file-preview | 锁定包 glTF WebGL、错误/取消、禁用/缓存重启 |
| ascii-tiles | 1.0.8 | appearance | 发布 Worker 夹具18通过；异常后自动清理1失败（基线缺陷），手动禁用/启用恢复；直接WebGL交互和11项打开恢复通过 |
| shallow-ocean | 1.0.17 | appearance | 源 Worker 绘制/回收；锁定包呈现与回退检查；待真实宿主 |
| multi-account | 1.0.110 | developer-tools | 锁定包市场下载/缓存；合成面板/路由/世代隔离；旧账号夹具22通过、共享终态轮询1失败（基线缺陷）；真实账号未验证 |
| spectral-drape | 1.0.2 | appearance | 锁定包 Worker 实时和暂停、30 控件、启停、缓存恢复 |

日志在 `artifacts/macos-port/plugin-offline/`、`artifacts/macos-port/{background,preview}-packages.log`；所有文件预览的发布包均未携带可供审计的 esbuild metafile，主预览 shell 仍通过可选依赖隔离检查。

## 尚未关闭的行为与发布缺口

- G2：独立官方实例停留在未登录页，未通过 Codex 工作区布局条件；完整 UI 注入尚未成功，不把最小 DOM 探针算成完成。
- G3：官方签名应用未发现合法的外部原生窗口控制通道；CSS 透明与实际 NSWindow 透明分别记录。
- G4：真实多账号登录、加密宿主凭证投影、模型/额度、发送取消及上下文延续没有用真实账户验收。
- 旧 `.ppt` 原生导出、实际 Finder/卸载对话框及注销后登录项行为、Windows runner、Developer ID/公证及 Mac 更新源均未通过完整验收。
- ASCII 异常后的自动清理，以及切回官方身份后 legacy account 的共享回合终态轮询，各保留一项失败；相应生产文件与 Windows 固定基线一致，未修改锁定发布插件来隐藏这些缺陷。
- 后台进程清理覆盖本产品启动的继承进程组；主动另建 session/group 的后代不受该机制约束，不能称为 Windows Job Object 的完全等价隔离。
- 本地 API 代理绕过属于共享实现，也改善 Windows 路径；Windows 原有就绪探测仍保留，实际 Windows 回归尚未运行。

## 当前开发产物

- 目录：`artifacts/macos-arm64-development-20261115/`
- `Code-Codex.app`、`Code-Codex-0.6.20-arm64.dmg`、`Code-Codex-0.6.20-arm64.zip`
- `verification.json` 保存包哈希和真实安装/回滚收据。
- 重复验证入口：`scripts/build-macos.sh --release`、`scripts/macos/test_package.py`、`scripts/macos/test_probe.py`、`cargo test --workspace --locked -- --test-threads=1`。
- 终态测试日志位于 `artifacts/macos-port/final-*.log`；已知插件失败保留在 `plugin-offline/*-recheck.log`。
