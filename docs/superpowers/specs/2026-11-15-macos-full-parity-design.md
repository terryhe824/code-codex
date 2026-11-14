# Code-Codex macOS 完整功能适配设计

日期：2026-11-15（Asia/Shanghai）

状态：设计草案，待用户批准；尚未实现或验证 macOS 运行。用户已确认目标为“完整复刻 Windows 版”。

## 1. 目标、基线与完成定义

在 macOS 上提供与固定 Windows 源码基线等价的用户功能，复用现有界面、插件机制和 Rust 服务。阶段交付用于验证依赖，不将核心功能原型算作完整完成。

- 上游：<https://github.com/Rice-dog/code-codex>。
- 固定提交：`03454a8d8a57c6588f902a33c9a16254b71ec009`，workspace 版本 `0.6.20`。移植期间不自动追赶移动的 main；上游更新单独评估、重新核对清单。
- 源码插件目录实际有 **29 项：17 个 appearance、10 个 file-preview、2 个 developer-tools**。README 的 27 项描述已不足以定义范围；以 `resources/plugin-definitions.json`、分发锁、实际路由和插件实现共同建立基线。
- 多账号/API 连接、模型选择、配额展示、用户启用的续接、上下文隔离等源码功能也列入验收，不能因 README 未列出而忽略。保留原有权限、身份和服务端配额约束。
- 该提交的 `optional-features.ts` 已启用 multi-account。分发锁有 29 包，其中 custom-background、multi-account、codex-startup-transition、spectral-drape 有 pendingPublicationAssets 记录。本轮 GitHub Release API 核查确认四项同名资产均已上传，大小和 API 提供的 SHA-256 与锁匹配；尚未下载重算字节哈希或执行插件。
- “等价”指用户操作、结果、状态持久化、错误处理与恢复行为等价；Finder、Command 键、系统登录项及 `.app` 安装方式采用 Mac 原生形式。系统控件不要求逐像素复制 Windows。
- 每项状态只能是：未实现、已实现待验证、通过、阻塞。不使用“跳过”抵消未完成项；已知平台限制保持为完整交付的缺口，变更目标需另行明确。
- 本机开发目标为 Apple Silicon ARM64。首版支持范围以本机 macOS 27.2.1 实测为准；macOS 13+ 为拟定兼容下限，须经构建和真机/CI 验证后方可宣称支持。Intel 不纳入首版架构验收，未来扩展不影响功能清单。

## 2. 已确认事实与关键未知项

### 仓库事实

- Rust 四个 crate：`launcher`、`cdp-client`、`context-resolver`、`workspace-service`；TypeScript 界面位于 `packages/explorer-ui`。
- 启动发现目前依赖 Windows AppX、`.exe` 与 `LOCALAPPDATA`；自动选择官方 App Server 依赖 Windows 包身份。见 `crates/launcher/src/discovery.rs`。
- `process_guard.rs` 非 Windows 分支并不完整：进程身份校验/attach 有直接失败分支，两个 listener owner 校验又直接返回成功。移植必须实现真实校验，不能沿用空实现作为通过。
- 原生透明窗口使用 Win32/DWM；原生 PowerPoint 导出使用 Windows COM；API 密钥库在非 Windows 平台返回不支持。
- CLI 中存在 run、attach、activate、diagnose、plugins、login-startup、host-updates 等路径；安装更新、启动冲突恢复、日志与卸载也是目标范围。

### 本机只读观察

- ARM64，macOS 27.2.1。
- 应用：`/Applications/ChatGPT.app`；bundle ID `com.openai.codex`，版本 `26.1111.21308`，build `27194`，声明最低 macOS 13.0。
- 运行时元数据为 `owl`，框架为 `Codex Framework.framework/Versions/157.0.8103.78`，不能假设为未经修改的 Electron。
- 内置 CLI 包版本 `0.169.0-alpha.11.1`、目标 `aarch64-apple-darwin`。`Contents/Resources/codex-cli/bin/codex` 为 wrapper，实际可执行文件在内部 `CodexCLI.app`。
- 签名描述显示 OpenAI Developer ID、hardened runtime 和 notarization ticket；仅查看描述，未作完整签名验证。
- 只读扫描到 Electron v1 fuse wire `010011001`：Node inspector、NODE_OPTIONS、RunAsNode 关闭，ASAR 完整性校验开启。这不等于 Chromium renderer CDP 被禁用。
- 本轮未启动、重启、附加或注入客户端，未读取账号凭证或聊天内容，未安装依赖。

### 必须实测的可行性门槛

G1：此 Owl build 是否接受 loopback renderer 调试参数，能否识别目标页面、建立绑定并加载现有 UI。

G2：实际 Mac 页面的布局、工作区/聊天身份和内部桥接合同是否足以支持文件树、模型/账号等功能；以能力检测为准，不能凭版本号放行。

G3：能否通过已验证的宿主能力实现真正的原生窗口透明和恢复。CSS 透明不算原生透明通过；普通辅助进程不能直接当作拥有宿主 NSWindow。

G4：多账号使用的官方后端接口、身份隔离和原生路由在 Mac 运行时是否成立。

若关键门槛失败：保存可复现证据和缺口，继续独立可推进模块；不擅自替换为独立网页、修改官方 app.asar、fuse 或重新签名官方客户端来宣称成功。需要改变整体产品路线时再向用户说明。

## 3. 方案比较与采用路线（Approaches）

| 方案 | 优点 | 代价与结论 |
| --- | --- | --- |
| 在现有代码增加 macOS 平台实现 | 最能保留现有界面、29 项插件和升级维护关系 | 需处理宿主能力差异；本设计采用，由实现方提出 |
| 重写为独立 Mac 伴随应用/网页 | 原生应用生命周期可控 | 无法等价复刻“嵌入 Codex 的同一界面”，不符合当前目标 |
| 在虚拟机中运行 Windows 版本 | 可保留 Windows 执行环境 | 不构成 macOS 原生适配，不采用 |

用户已选择完整功能目标；具体平台实现由本设计提出，设计批准后执行。

## 4. 平台实现边界

保留共享协议和 UI 生命周期。按现有模块逐项抽取真正的平台操作到 `platform/macos` 与现有 Windows 实现；不做无关重构，不一次性改写全部 launcher。小型原生 helper 只用于 Rust 无法合理覆盖的系统能力，不重写 TypeScript 界面。

### 4.1 应用发现、启动与进程身份

- 按 bundle ID 查找并解析 `Info.plist`，支持系统及用户 Applications 路径和显式指定安装。核对主程序、资源、CLI 和签名身份；不依赖应用显示名称。
- 路径按 macOS 大小写和文件身份语义处理，包含中文、空格、Unicode 组合字符、移动安装和符号链接边界测试。
- 运行时调试端口只绑定 loopback，校验端口归属 PID、可执行路径、创建时间和用户身份；重连/激活时重新验证，拒绝未知、复用或替换的进程。
- 不复用非 Windows `Ok(())` 占位。具体进程查询使用系统原生进程/socket 信息；遇到无法确定的所有权则失败并显示原因，不放宽为任意本地端口。
- 已运行的官方会话按原有产品流程提供正常退出并重开选项；取消保留当前会话，超时不强制杀死。崩溃恢复、观察已附加进程与终止自己创建的 helper 分开处理。
- 原型实测优先使用独立临时 profile/可抛弃工作区；不能由隔离测试推断已验证真实账号。当前活跃聊天需要关闭/重开时，留到可审阅的具体操作再确认。

### 4.2 文件服务、工作区与 Mac 交互

- 保留工作区授权根、capability token、epoch、路径防逃逸、文件大小限制、并发编辑冲突和文件 watcher 合同。
- 支持 Finder 拖入文件/目录、树内移动、创建、重命名、删除、复制路径、刷新、打开与定位；沿用上游删除语义并明确提示，不悄悄改成另一种行为。
- 保留现有外部拖入的 File System Handle/Entry/File 字节导入路线及数量、深度、大小限制，在已授权工作区下暂存并提交；实测 Finder 输入，不要求 renderer 获得任意绝对路径。浏览器显示名或伪造 renderer 路径不作为本机路径授权依据。
- Cmd/Ctrl 快捷键按平台适配；验证中文输入法、Retina、全屏、多窗口、窗口缩放和系统深浅色。
- 文件系统要实测默认不区分大小写与区分大小写 APFS 上的仅改大小写重命名、跨设备移动、硬链接/符号链接和文件身份。上游删除是永久删除，不承诺原项目不存在的回收站语义；保留明确确认。
- 前端 Windows 专属保留文件名检查改为平台规则，不能禁止 macOS 本来合法的名称；后端继续独立校验，不靠前端保证安全。
- 系统语言检测、默认浏览器打开官方登录链接、顶部菜单和重启入口分别替换 Windows 实现，不能只替换可见文案。

### 4.3 存储、安装、更新与卸载

- 应用包与可变数据分离：默认应用包 `~/Applications/Code-Codex.app`；持久数据 `~/Library/Application Support/Code-Codex`；日志 `~/Library/Logs/Code-Codex`；可重建缓存 `~/Library/Caches/Code-Codex`。自定义固定本地安装目录保留，应用移动后按 bundle ID 和安装标记重新定位。
- 插件、用户图片/视频、设置、已下载资源保留版本和完整性验证；不可写入只读 app bundle 或官方配置目录。按原有合同导入和迁移，失败可恢复。
- 交付 `.app`、DMG、ZIP，以及可明确选择保留/清除产品数据的卸载入口。Windows MSI 的软件分发用途映射为 Mac 原生安装包/流程，不运行 Windows 安装脚本。
- 更新包使用独立 macOS 发布资产和发布通道，不能下载上游 Windows EXE。来源、大小、SHA-256、产品/平台/架构、签名身份均验证；原子切换和失败回滚，保留当前及上一有效版本。
- 如有插件修改，生成独立版本/分发锁，不在旧上游版本名下替换字节；缓存有效性、离线导入、按需下载与启用语义保持。
- 实现 opt-in 登录项和宿主版本变化检查，保存选择、系统撤销后不偷偷恢复；卸载撤销本产品项。
- 本地构建验证与面向他人的正式分发分开。正式分发需要项目所有者提供 Developer ID 签名/公证条件和可管理的 Release 仓库；未具备时交付本地构建及发布脚本，正式发布项仍记为未完成，不索取或展示秘密。

### 4.4 外观、启动动画与宿主恢复

- 复用 WebGL/Canvas/Worker 插件及设置，验证暂停、减少动态效果、隐藏后台、质量档位、素材迁移和深色模式恢复。
- 原生窗口透明需要通过 G3，另验开启/关闭、窗口重建、全屏、退出与异常恢复；不把背景插件或 surface opacity 当作替代。
- 启动动画保持“不阻塞官方启动、缺包可继续”的行为；真实首帧、首屏加载、背景来源、用户视频和退出动画均验收。

### 4.5 Office 与媒体预览

- 优先保留现有本地 Worker/WASM 和 JS 渲染器，实测 PDF、CSV、图片、音视频、Notebook、图表及 glTF/GLB。
- 上游原生 PowerPoint 导出仅服务旧版 `.ppt`；`.pptx` 使用 JS 渲染器，DOCX/XLSX 也有各自本地实现。逐格式保存其功能基线。
- 旧版 `.ppt` 的 PowerPoint 原生预览改为经测试可用的 Mac Office 自动化导出适配器，保留源文件只读、超时、取消和产物清理；系统自动化许可由正常 UI 申请。
- 第一步须验证 Mac PowerPoint 是否提供所需导出能力和保真度。若不具备，记录 native-PowerPoint 子项阻塞；LibreOffice/通用渲染器只作为需明确标注的候选，不自动认定保真等价。
- 未安装 Office 时，保持上游可用的通用预览与准确错误提示。分别测试有 Office 和无 Office 环境；不存在的外部依赖不记为测试通过。

### 4.6 多账号与 API 工具

- 覆盖账号列表、登录/注销、官方与 API 选择、模型和 reasoning effort、配额展示、连接诊断、原生 composer 生命周期、流式响应、取消、上下文交接及用户启用的自动续接。
- Windows Credential Manager 对应为 macOS Keychain；新增凭证只走原生受控存储，renderer 与日志不包含 secret。官方登录使用经验证的后端能力，不能把 Windows auth.json 布局直接假设为 Mac 合同。
- 独立后端显式设置隔离的 CODEX_HOME，保留 HOME/PATH 等必要 Mac 环境并验证宿主兼容；不改用户全局环境或默认认证文件。
- 跨账号、provider、模型的上下文隔离遵循上游合同；不把权限错误、网络错误或取消解释为可自动重发条件，不绕过服务端配额。
- UI/协议测试先用合成数据。实际账号登录、发送请求或消耗额度的验收需要用户选择测试账号和一次具体操作；全功能目标本身不授权切换当前账号或发送真实消息。

## 5. 功能与验收清单

每一行的“通过”必须附构建提交、宿主版本、系统/架构、操作步骤、实际结果和证据路径。当前所有实现项均为未实现/待验证。

| 功能组 | 最少验收条件 |
| --- | --- |
| 启动与连接 | run/attach/activate/diagnose；非标准应用名称；多安装选择；正确身份放行、错误身份/非 loopback 拒绝 |
| 工作区与树 | 聊天切换正确切换根；新旧 epoch 隔离；文件增删改移；拖入/拖动；中文路径；外部文件变更自动更新 |
| 编辑与标签 | 代码高亮、缩略图、打开关闭/切换标签、修改保存/冲突、快捷键和输入法 |
| 文件预览 10 项 | 下表全部插件逐项打开代表性文件；损坏、过大、不支持格式的错误正确；异步退出无残留 |
| 外观 17 项 | 下表全部插件逐项启停、参数保存、主题恢复、重启与窗口重建；透明和启动动画单独实测 |
| Git | 分支、历史、改动文件和 diff 正确，仅执行上游允许的只读操作 |
| 多账号 | 选择/隔离/登录与 API 密钥库；模型/配额/续接/取消/恢复；原生权限和身份约束保留 |
| 插件市场 | 分类、置顶、下载、校验、启用、参数、缓存、离线导入、升级后偏好保留 |
| 安装与生命周期 | 干净安装、中文/自定义路径、移动、升级、失败回滚、正常卸载/清除、登录项启停与撤销 |
| 诊断与宿主更新 | 可读错误、脱敏诊断、日志限制、上游 14 类宿主合同检查、升级后能力重新验证 |
| 发布与回归 | ARM64 包可安装；正式签名公证发布另有证据；Windows 原有功能回归；许可证随包保留 |

插件清单（固定提交的定义；对应分发资产可用性仍须逐项核对）：

| 分类 | 插件 ID |
| --- | --- |
| 外观 | glow-horizon、black-hole、heavenly-cloud、aurora-ionosphere、milky-way、mountain、cloud-train、blinking-squares、particle-image、pixel-sculpt、custom-background、surface-opacity、transparent-background、codex-startup-transition、ascii-tiles、shallow-ocean、spectral-drape |
| 文件预览 | markdown-preview、csv-preview、diagram-preview、image-preview、video-preview、pdf-preview、audio-preview、office-preview、notebook-preview、model-preview |
| 工具 | git-history、multi-account |

## 6. 阶段与验证顺序

1. **宿主可行性**：编译/依赖基线、G1/G2/G3/G4 能力证据；先把不可行的宿主假设暴露出来。验证不修改官方 bundle、不干扰活跃会话。
2. **平台基础与核心 UI**：发现、路径、身份校验、服务、文件树/编辑/拖放/Finder。原型验收只说明主链路可用。
3. **全插件与账号**：29 项逐项实测，先验证原生透明/Office/账号高风险合同，其余可并行；保留每项缺口。
4. **安装与更新**：app/DMG/ZIP、安装选择、数据迁移、更新/回滚、登录项、卸载及宿主变更诊断。
5. **完整回归与交付**：Mac 真实宿主测试、可用 Windows runner 回归、发布验证、用户操作说明和最终缺口表。不能以模拟测试代替真实宿主通过。

实施计划在设计批准后编写。先利用已有架构检查、前端合同测试及 Rust 测试；新增测试集中在真实 Mac 边界和回归场景，不照抄实现。构建与测试命令以仓库现有脚本为准，尚未在本轮执行。

## 7. 交付物、责任与待验证条件

- 本地代码路径：`/Users/terry/Documents/code-codex-macos`；分支 `codex/macos-full-parity`；保留上游追踪。源码采用 sparse checkout 暂不下载现成 Windows 安装包，发布阶段按需要取得锁定资产。
- 最终交付：可复现源码、构建/打包脚本、Mac 安装产物、逐项功能证据、已知问题、使用/更新/卸载说明。
- 实现方负责接口细节、原生 API 选择、内部模块划分和测试夹具；这些不需要逐条询问用户。
- 需要后续环境条件：可退出/重开的测试客户端；Mac Office（验证其原生预览分支）；真实账号/API 测试材料；Windows 测试环境；正式签名和发布渠道。缺少时继续可独立完成的工作，并单独列出未验证项目。
- 不预先承诺全部插件可在当前 Owl build 无条件运行，也不在可行性验证前给出完整工期承诺。任何关键能力不存在都要明确报告。

## 8. 证据与参考

- [固定源码基线](https://github.com/Rice-dog/code-codex/tree/03454a8d8a57c6588f902a33c9a16254b71ec009)
- 仓库文件：`resources/plugin-definitions.json`、`resources/plugin-distribution.lock.json`、`resources/host-update-dependencies.json`、`crates/launcher/src/discovery.rs`、`process_guard.rs`、`accounts/api.rs`、`platform/windows/window.rs`、`crates/workspace-service/src/preview/native_powerpoint.rs`。
- 本机证据：`uname -m`、`sw_vers`、`mdfind`、`Info.plist`、`owl-electron-app.json`、内置 CLI manifest、`file`、`codesign -d` 描述和只读 fuse 扫描。尚无运行验证证据。
- [Electron 官方 fuse 定义](https://github.com/electron/fuses/blob/main/src/config.ts)及[说明](https://www.electronjs.org/docs/latest/tutorial/fuses)：Node inspector 开关不能当作 renderer CDP 成败证据。
- [Electron 调试开关](https://www.electronjs.org/docs/latest/api/command-line-switches)：通用开关说明不是当前 Owl 客户端支持承诺。
- [Apple 公证与分发](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution)：正式 Mac 分发的签名和公证流程。
- 四项发布资产元数据：GitHub Release tag `plugin-assets-2026-11-01`（custom-background）、`plugin-assets-2026-11-10`（multi-account）、`plugin-assets-2026-11-11`（startup-transition、spectral-drape），2026-11-15 查阅；这里只核对 API 元数据，不代表实际下载验收。

## 9. 设计审阅记录

2026-11-15，独立 macOS/Rust 实现审阅代理完成 builder check，结论为 `nothing important left`。G1–G4、Office 与分发条件被明确列为后续验证任务，未发现需要用户补充的实施前范围决策。此审阅不等于代码运行、兼容性或功能验收通过。
