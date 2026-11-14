# macOS Full Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 Apple Silicon macOS 实现固定 Windows 源码基线的完整功能，逐项验证，不把原型或不支持占位当作完成。

**Architecture:** 保留 Rust 四个 crate 与 TypeScript UI，增加平台模块。真实宿主能力、系统集成、文件操作、插件与发布分别验收。实现阶段使用已创建的专用 checkout 和 `codex/macos-full-parity` 分支。

**Tech Stack:** Rust、TypeScript/Node 22、macOS 系统 API、现有插件分发锁。

**Spec:** `docs/superpowers/specs/2026-11-15-macos-full-parity-design.md`（用户于 2026-11-15 已批准实施）

## Global Constraints

- 基线 `03454a8d8a57c6588f902a33c9a16254b71ec009`，29 个插件，完整功能目标不变。
- 首版 ARM64；本机 macOS 27.2.1；13+ 仅在实测后承诺。
- 不修改官方 bundle/ASAR/fuse，不中断当前活跃聊天；隔离的临时 profile 可用于验证。
- 不读取真实凭证/历史来完成合成测试；真实账号发送和发布有各自的外部条件。
- 保留 Windows 分支、路径防逃逸、loopback/进程身份和插件完整性校验。
- 未验证的透明窗口、Office 原生分支、账号和正式发布必须列为缺口。

## Review Focus

- 端口被另一进程占用或 PID 复用：必须拒绝，任务 3 的真实 socket 测试覆盖。
- macOS 默认不区分大小写文件系统上的大小写重命名：正确执行而不覆盖其他文件，任务 4 覆盖。
- 带中文/空格的应用和工作区路径：参数不经 shell 拼接，任务 2、4、7 覆盖。
- 插件/更新下载失败或哈希不符：不执行、不破坏原版本，任务 6、7 覆盖。
- 退出、取消与账号切换后的迟到结果：不写入另一个工作区/身份，任务 5、6 覆盖。

## Task 1: 可复现构建与真实宿主能力检查

**Files:** `scripts/macos/probe-host.py`、`scripts/build-macos.sh`、`artifacts/macos-port/`（证据，不入产品包）。
**Interfaces:** 产出 JSON 能力报告（版本、CDP、目标页面、错误及隔离状态），不产出认证数据。构建产出 `target/debug/code-codex` 与现有 `packages/explorer-ui/dist/`。

- [ ] 运行未修改的架构检查；安装缺少的 Rust 工具链与锁定 npm 依赖，记录原始失败。
- [ ] 运行现有 UI build 与 `cargo check --workspace --locked`，只修复阻止 macOS 编译的具体错误。
- [ ] 编写可抛弃 profile 的有界探测，验证自己的 PID/loopback socket，只读取自己的页面元信息并结束自己创建的进程；证明现有会话 PID 未退出。
- [ ] 记录 G1–G4 支持/失败/未知。G1 成功后才测试完整 UI 注入；失败仍推进独立可测模块。
- [ ] 验证：构建命令成功，或报告准确缺失资源；隔离探测报告必须区分应用启动与 CDP/注入成功。

## Task 2: Mac 应用发现与官方 CLI 解析

**Files:** `crates/launcher/src/discovery.rs`、`crates/launcher/src/discovery/macos.rs`、`crates/launcher/src/bootstrap.rs`。
**Interfaces:** 保持 `discover_codex(...) -> Result<CodexInstallation, DiscoveryError>`、`discover_app_server_source(...)`；新增 `DiscoverySource::MacApplication`。平台模块在 discovery 内部私有，不要求改 main 模块声明。

- [ ] 先加测试：非标准 app 名、错误 bundle ID、缺失主程序、中文空格路径、stable/beta 过滤、资源目录与嵌入 CLI 实际路径。
- [ ] 观察未实现时失败，再实现应用 metadata/签名身份核对和 App Server 解析；显式开发 override 保留现有语义。
- [ ] 修正 bootstrap 的 app.asar 位置选择，不改变 Windows 路径。
- [ ] 验证：`cargo test -p code-codex --bin code-codex discovery -- --test-threads=1`；只读 diagnose 命中真实本机 app，不能为身份不明的包报告可信。

## Task 3: Mac 进程与 socket 身份、附加生命周期

**Files:** `crates/launcher/src/process_guard.rs`、`crates/launcher/src/process_guard/macos.rs`。
**Interfaces:** 保持 `verify_process_identity`、`verify_listener_owner`、`verify_listener_executable`、`AttachedProcess`、`is_executable_running`、debug port discovery 的现有调用合同；Mac 进程身份包含 PID、启动时间、用户和可执行路径。

- [ ] 先写真实进程/socket 测试：当前进程 loopback 可验证、错误路径/端口拒绝、关闭 socket 拒绝、非 loopback 拒绝、已退出进程不跟随新 PID。
- [ ] 实现系统进程/socket 观察与 fail-closed 分支，替换 Mac 的空成功/空失败；不放宽其他平台。
- [ ] 验证 attach 观察对象不会终止被观察进程，launch-owned 子进程可清理；处理子进程 listener 的受限祖先关系。
- [ ] 验证：`cargo test -p code-codex --bin code-codex process_guard -- --test-threads=1`；G1 成功后接入自己的探测进程。

## Task 4: 文件路径、存储与 Finder 基础

**Files:** `crates/workspace-service/src/application_paths.rs`、`mutation.rs`、`crates/launcher/src/bridge.rs`、`packages/explorer-ui/src/features/file-operations/context-menu.ts` 及对应测试。
**Interfaces:** 保留 SettingsStore/application_paths 和 Workspace 公开接口；macOS 默认数据根 `~/Library/Application Support/Code-Codex`、日志 `~/Library/Logs/Code-Codex`、缓存 `~/Library/Caches/Code-Codex`。

- [ ] 先写临时目录测试，覆盖大小写重命名、已有目标不覆盖、中文/Unicode、链接逃逸和跨目录操作。
- [ ] 实现 Mac 文件系统语义、平台文件名校验、数据/缓存/日志位置与 Finder reveal；避免可变数据写入 app bundle。
- [ ] 验证：`cargo test -p workspace-service --locked` 和对应文件操作脚本；不存在可用区分大小写卷时明确未验该分支。

## Task 5: Mac 系统与账号基础集成

**Files:** `crates/launcher/src/accounts/{api,credentials,worker,mod}.rs`、`localization.rs`、`login_startup.rs`、`menu_restart.rs`、`startup_conflict.rs` 和新的平台小模块。
**Interfaces:** 保留现有 bridge JSON/错误合同；Keychain 存取接入现有 `store_key/read_key/delete` 实际接口；不向 renderer 泄露 secret。

- [ ] 测试先覆盖环境白名单、URL scheme/参数校验、语言检测、登录项注册状态/撤销和 restart 取消。
- [ ] 实现 Mac 浏览器打开、HOME/SHELL/TMPDIR 环境、原生密钥库与默认语言；使用合成独立 Keychain 服务名测试存取删，不读取用户已有项。
- [ ] 实现明确 opt-in 登录项和已有实例退出确认；当前会话不用于破坏性测试。
- [ ] 验证：账号/登录项相关 Rust 与 JS 合同测试；真实账号登录/发送留为单独具体验收。

## Task 6: 全插件、原生窗口与 Office

**Files:** `resources/plugin-{definitions,distribution.lock}.json`、`crates/launcher/src/platform/macos/`、`crates/workspace-service/src/preview/native_powerpoint.rs`、`docs/macos-parity.md`。
**Interfaces:** 保留 `window_transparency_set` 和 `.ppt` 导出合同；改动插件必须采用新版本/哈希。

- [ ] 生成 29 项逐项矩阵；下载锁定资产并重算哈希，不执行不匹配资产。
- [ ] 逐插件验证初始化/启用/参数/停用/恢复/离线；运行已有 UI 合同和类型检查。
- [ ] 真正原生透明需要宿主能力证据；无合法控制通道保持阻塞，不能用 CSS 替代。`.ppt` 先检验已安装 Office 的导出能力，再实现有界原生适配器。
- [ ] G2/G4 成立后验证模型/配额/取消/上下文，实际账号行为需要用户指定测试材料。
- [ ] 验证：每个插件一条实际证据或准确缺口，不允许把静态定义数量算作功能通过数。

## Task 7: 安装、更新与回滚

**Files:** `scripts/macos/package.sh`、`scripts/macos/uninstall.sh`、Mac 平台安装/更新路由、`.github/workflows/macos.yml`。
**Interfaces:** `.app` 包内不可变程序，用户 Library 下可变资源；只消费 macOS/ARM64 包且校验产品/架构/签名/哈希。

- [ ] 用临时安装根先测中文路径、已有版本、失败更新回滚、保留数据卸载与显式清除产品数据。
- [ ] 实现本地 `.app`/DMG/ZIP 构建、默认用户 Applications 目标和独立卸载入口，保留当前及前一有效版本。
- [ ] 增加 ARM64 CI 构建/测试；正式签名公证与远端 feed 未配置时明确不可发布/不可在线更新，不回退 Windows feed。
- [ ] 验证包结构、`--help`/只读 diagnose 和本地安装生命周期；真正发布等待管理的 Release 仓库与签名条件。

## Task 8: 综合审阅与交付

**Files:** `docs/macos-parity.md`、`docs/macos-install.md`、所有前述改动。
**Interfaces:** 最终矩阵记录提交、宿主版本、系统/架构、命令/步骤与结果。

- [ ] 跑 `cargo test --workspace --locked`、UI typecheck/build、架构/i18n、相关 Node 合同测试；保存失败并逐个修复相关问题。
- [ ] 用真实 Mac 宿主验收主流程及 29 插件，Windows CI 没运行则不能宣称 Windows 回归通过。
- [ ] 独立审阅整体 diff，处理影响正确性/安全/数据的发现后重验。
- [ ] 保留工作提交、构建产物和使用说明；完整目标只有全部必需项真实通过才完成，外部条件阻塞项单列。
