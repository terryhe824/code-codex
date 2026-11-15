# Code-Codex macOS 开发版

此分支适配 Apple Silicon。目标是 Windows 基线的完整功能；当前仍处于开发包验收阶段，不能视为完整复刻的正式发布版。真实宿主已测 macOS 27.2.1、官方应用 26.1111.21308；macOS 15.8.2 ARM64 远端构建及自动化回归也通过。最低系统声明为 macOS 13.0，但 macOS 13 实机运行仍待验。

## 本次已生成的开发包

当前包位于 `artifacts/macos-arm64-development-accepted-20261115/`，包含 `.app`、DMG、ZIP 和 `verification.json`，源码提交为 `3f0e92a`。其代码与双平台 CI 通过的 `7c7e219` 相同，后续差异仅为验收文档。

完整 release/UI 构建通过；包签名、ARM64 与最低系统声明、ZIP 解压后的应用、DMG 只读挂载后的应用均验证通过，两个归档中的应用与原包的原生内容指纹一致。新包核心在临时中文目录通过 10 项安装、升级、错误哈希拒绝、运行进程保护和回滚检查。旧目录保留历史证据，不应作为当前开发包入口。

当前账号的一次已授权请求响应和 Finder 普通文件、嵌套目录拖入已通过真实验收；双账号、原生透明、旧 PPT 自动预览等缺口见 `macos-parity.md`。当前优先验收本地开发包，正式签名、公证和在线更新另行完成。

## 从源码构建

需要当前稳定 Rust、Node.js 22、Xcode Command Line Tools。官方应用独立安装，本项目不会修改其 bundle、ASAR 或签名。

```sh
bash scripts/build-macos.sh --release
python3 scripts/macos/package.py --binary target/release/code-codex --output artifacts/macos-release --version 0.6.20
```

产物为 `Code-Codex.app`、ARM64 ZIP 和 DMG，使用本地 ad-hoc 签名。它们尚未使用 Developer ID 签名或经过 Apple 公证。不要把本地开发产物作为正式签名版本分发。

Bundle 内 GUI 为 `Contents/MacOS/Code-Codex`，命令行核心为 `Contents/MacOS/code-codex-core`。两者不能只靠大小写区分，因为常用 APFS 卷不区分大小写。程序包内只放不可变资源，用户数据在 Library 下。

## 安装和回滚

核心提供 `mac-install`、`mac-rollback`、`mac-package-digest` 和 `mac-uninstall`。默认目标为当前用户的 `~/Applications/Code-Codex.app`。正式签名信任尚未配置时，仅可用显式 `--allow-ad-hoc` 验收本地开发包。安装可传 `--sha256` 固定来源内容；错误指纹会在替换前拒绝。

```sh
target/release/code-codex mac-package-digest --source artifacts/macos-release/Code-Codex.app
target/release/code-codex mac-install --source artifacts/macos-release/Code-Codex.app --allow-ad-hoc
target/release/code-codex mac-rollback --allow-ad-hoc
```

安装校验具体行为和本地生命周期结果以 `macos-parity.md` 的最新记录为准。在线 Mac 发布通道尚未配置，应用不会使用 Windows 安装程序替代 Mac 更新。

## 启动和登录项

打开安装后的 `Code-Codex.app`。如果官方应用已经运行，交互式启动需要用户确认后才能请求正常退出；取消或超时不得强制终止。当前聊天会话不用于破坏性验收。

登录启动默认关闭。通过应用设置或核心的 `login-startup --enable` 明确启用；`login-startup --disable` 撤销。排队的登录启动遇到已经运行的官方应用会直接跳过。实际注销后重新登录的系统验收仍待完成。

## 数据与卸载

- 数据：`~/Library/Application Support/Code-Codex`
- 缓存：`~/Library/Caches/Code-Codex`
- 日志：`~/Library/Logs/Code-Codex`
- 凭证：用户 Keychain 中按独立账号目录隔离的项

默认卸载保留产品数据。显式清理选项需要单独确认，并只能作用于本产品拥有的数据及对应凭证；官方客户端和它的账户资料不属于卸载范围。

卸载和登录项操作依据系统用户目录，不能仅靠设置临时 `HOME` 将它们隔离。实际卸载验收需单独安排；不要将安装/回滚临时目录测试扩展成对当前用户数据的卸载测试。当前已用受删除保护的临时应用副本运行真实卸载入口，约 60 秒后返回 `cancelled=true`、`uninstalled=false`，内容指纹与现有进程均未变化。没有手动点击确认或窗口可见性证据，因此不计作手动取消、保留数据卸载或清除数据卸载完成。

## 只读诊断和隔离测试

```sh
target/release/code-codex diagnose
target/release/code-codex login-startup
python3 scripts/macos/probe-host.py --core target/release/code-codex --report artifacts/macos-port/host-probe.json
```

隔离探测创建临时 HOME、CODEX_HOME、浏览器 profile 和测试工作区，只清理它自己创建的进程。未登录窗口无法证明 Codex 工作区、真实账户路由、原生透明或插件在真实宿主中的兼容性。普通 Chrome 合成测试与真实宿主验收分别记录。

## 手动登录验收窗口

需要用户操作登录时，使用持续会话入口。选择尚不存在的会话目录；脚本会生成独立 HOME、CODEX_HOME、CODEX_SQLITE_HOME、浏览器 profile 和合成工作区，并把带黄色标记的测试窗口置于前台。

```sh
python3 scripts/macos/manual-session.py \
  --session "$HOME/Library/Application Support/Code-Codex-Mac-Testing/my-new-session" \
  --core target/release/code-codex --background
```

使用独立控制命令查询或管理刚创建的会话，而不是在终端输入裸命令：

```sh
python3 scripts/macos/manual-session.py \
  --session "$HOME/Library/Application Support/Code-Codex-Mac-Testing/my-new-session" \
  --command status
```

`status` 返回控制器和进程状态；用户登录后用 `--command attach` 启动文件界面验收，`--command detach` 结束附加核心，`--command stop` 结束这个测试会话拥有的进程组。已登录并运行中的验收窗口不需要重复创建。默认使用新 profile；显式续用旧 profile 需要先停止对应会话，并使用 `--reuse-profile-from` 及指定的签名应用副本。测试仍运行于同一个 macOS 用户，系统钥匙串并非独立账户。

Finder 拖入夹具应放在 Documents 下的独立测试目录。宿主会拒绝部分 Library/应用数据目录，不能通过改用另一个文件接口绕过拒绝。

登录后只使用新建的合成对话和工作区。页面观察不读取历史对话文本；私有 host/core 日志不应整体输出。直接打开应用后附加并不能证明启动时的原生账号通道，需另外验证。真实发送任务必须有明确的测试内容。
