# Code-Codex macOS 开发版

此分支适配 Apple Silicon。目标是 Windows 基线的完整功能；当前仍处于实施和验收阶段，不能视为完整复刻的正式发布版。已测主机为 macOS 27.2.1，官方应用 26.1111.21308；其他 macOS 版本尚未实测。

## 本次已生成的开发包

`artifacts/macos-arm64-development-20261115/` 包含 `.app`、DMG、ZIP 和 `verification.json`。已用真实release二进制在临时中文路径验证安装、错误哈希拒绝、升级和回滚。它仍有 `macos-parity.md` 所列功能缺口，尚未经过真实账号或原生透明验收。

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
  --core target/release/code-codex
```

在该终端输入 `status` 仅返回页面结构计数；用户登录后输入 `attach` 启动文件界面验收，`detach` 结束附加核心，`stop` 或终端中断结束这个测试会话拥有的进程组。测试资料和登录状态保留在新会话目录，便于后续复验；不复用已有 profile。它仍运行于同一个 macOS 用户，系统钥匙串并非独立账户。

登录后只使用新建的合成对话和工作区。页面观察不读取历史对话文本；私有 host/core 日志不应整体输出。直接打开应用后附加并不能证明启动时的原生账号通道，需另外验证。真实发送任务必须有明确的测试内容。
