# Code-Codex macOS 正式分发

正式产物面向 Apple Silicon，声明的最低系统版本为 macOS 13.0。macOS 13.0 上的运行验收仍待完成。项目所有者需提供有效的 Developer ID Application 证书、对应的 Team ID、同一团队的 `notarytool` 钥匙串配置名称，以及可管理的发布仓库。使用带 `vtool`、`notarytool` 和 `stapler` 的 Xcode 工具链。签名使用钥匙串中的私钥和 Apple 时间戳服务。Apple 的证书指纹和 Team ID 可用以下命令查看，公证凭证通过钥匙串配置名称传入。 [Apple 分发签名文档](https://developer.apple.com/documentation/xcode/creating-distribution-signed-code-for-the-mac) · [Apple 公证手册](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)

```sh
security find-identity -v -p codesigning
```

## 构建与检查

以下以 0.6.20 为例。用新的开发打包和发布工作目录。也可直接检查已有的项目开发包。开发构建中的核心程序会输出自身版本，须与打包版本一致。

```sh
version=0.6.20
dev="artifacts/code-codex-$version-development"
work="artifacts/code-codex-$version-distribution"

bash scripts/build-macos.sh --release
python3 scripts/macos/package.py --binary target/release/code-codex \
  --output "$dev" --version "$version"
python3 scripts/macos/release.py check --app "$dev/Code-Codex.app" --version "$version"
```

`check` 读取项目包标识、版本、由 `package.py` 写入的标准存储布局标记、签名完整性和两个可执行文件的 Mach-O 架构与部署目标，不启动应用或访问公证服务。开发产物使用 ad-hoc 签名。

## 签名、提交和完成

将示例值替换为项目所有者的 Team ID、Developer ID Application 证书指纹和已存入钥匙串的公证配置名称。为正式发布使用拥有权限的项目来源和钥匙串。命令仅处理单独构建的项目包，签名作用于新工作目录内的副本。

```sh
team_id='<10-character-team-id>'
identity='<40-character-certificate-fingerprint>'
notary_profile='<notarytool-keychain-profile>'

python3 scripts/macos/release.py prepare --app "$dev/Code-Codex.app" \
  --version "$version" --output "$work" --team-id "$team_id" --identity "$identity"

python3 scripts/macos/release.py submit --work "$work" --keychain-profile "$notary_profile"

python3 scripts/macos/release.py finish --work "$work" --keychain-profile "$notary_profile"
```

- `prepare` 从内到外签署核心、应用包和 DMG，校验 Apple 证书链、签名团队与时间戳，并校验应用及其核心启用了 Hardened Runtime。工作目录内的 `notarization.json` 记录候选件及 SHA-256。
- `submit` 验证工作目录内的临时副本，上传一次并记录 Apple 提供的提交 UUID。若上传结果不确定且记录中没有 UUID，先查看历史记录，再用 `finish --submission-id` 继续。恢复时会将 Apple 公证日志的 SHA-256 与候选 DMG 比对。一次提交尝试之后，同一工作目录会拒绝再次上传。
- `finish` 每次查询一次 Apple 状态。`In Progress` 返回退出码 2，可稍后再次运行。`Invalid` 和 `Rejected` 写入失败状态，使用 Apple 日志定位原因。Apple 确认接受后，脚本在附加 Ticket 前，将最终暂存区的 DMG 与本次获批的候选件记录逐项核对。随后为 DMG 和独立应用附加并验证 Ticket，从该应用生成 ZIP，并解压验证其中的签名及 Ticket。若附加或验证失败，保留已接受的记录，可重新运行 `finish`。
- `dist/` 只有在验证通过后才出现，包含独立的 `Code-Codex.app`、DMG、ZIP、Apple 公证日志、`SHA256SUMS` 和标为 `release-ready` 的 `release.json`。DMG 使用外层 Ticket，ZIP 内的应用携带自己的 Ticket。校验和针对完成后的 DMG 和 ZIP，文件大小、签名团队、提交 UUID 及代码目录哈希也由 `release.json` 记录。现有的工作目录和 `dist/` 均拒绝覆盖。 [Apple 打包文档](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution)

查询提交历史和诊断日志的命令如下。恢复 ID 时以同一团队的配置查询。

```sh
xcrun notarytool history --keychain-profile "$notary_profile"
xcrun notarytool log '<submission-uuid>' --keychain-profile "$notary_profile"
python3 scripts/macos/release.py finish --work "$work" --keychain-profile "$notary_profile" \
  --submission-id '<submission-uuid>'
```

校验最终档案后，由项目所有者在受管理的发布仓库审核并发布。脚本产出本地交付件。产品功能及真实宿主验收以 [macOS 验收记录](macos-parity.md) 为准。
