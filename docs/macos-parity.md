# macOS 适配验收记录

## 中文输入法补充验收（2026-11-15）

- 文件筛选框及文件编辑区此前会处理输入法组合期间的 Esc，造成筛选被清空或切回聊天；编辑区的保存快捷键也没有避让组合状态。现对 `isComposing` 及组合结束键 `keyCode=229` 保留输入法控制权，正常 Cmd/Ctrl-S、Esc 和筛选方向键保持原行为。
- `node scripts/test-workspace-ime.mjs` 使用生产组件和独立 Chromium，13 项检查通过；去掉修复时 10 项组合键断言失败。没有调用真实文件写入或模型消息，也未把合成键盘事件计为 macOS 输入法候选窗通过。该检查加入 Windows contracts 工作流。
- 73 项共享合同、TypeScript 类型检查及完整 release/UI 构建通过。既有 `test-code-minimap.mjs` 在“Native caret navigation auto-scrolls”断言失败，去掉本次修复后也同样失败；此整套回归未通过，原断言保留，不能据此声称编辑器全部验收完成。
- 新开发包候选位于 `artifacts/macos-arm64-ime-20261115/`。真实中文候选词取消、提交后保存、窗口缩放/全屏和主题仍待用户验收。此前双平台 CI 对应 `7c7e219`，不能作为本次修改的远端回归结果。

目标：完整复刻 Windows 基线 `03454a8`，不是核心功能演示。

当前状态：已有 ARM64 本地开发包；已完成用户手动登录后的真实窗口文件与插件主要路径验收。同一源码提交的 Windows/Mac 配置内自动化回归均已通过。完整 Windows 功能对齐仍未完成，真实多账号消息链路、原生透明、旧版 PPT 自动预览、正式分发及真实系统验收仍有缺口。

## 继续补齐与用户复验（2026-11-15）

- 用户暂时没有第二个测试账号；通过生产界面发起的子账号授权已取消。本轮未发送新的模型消息。空连接清理暴露了隔离测试 HOME 没有默认钥匙串引用的问题。启动器修复已提交 `b930ab7`，官方 CLI 和合成原生钥匙串测试均通过；真实用户的钥匙串偏好未变，也未复制凭证。独立实例已用修正后的启动器重开。
- 官方模型菜单实测有 9 个模型选项，完成切换、关闭后重新打开及恢复原模型。收据 `artifacts/macos-port/live-model-menu.json`。早先测试错误地依赖按钮文案，保留失败收据，最终以重新打开后的原生菜单选中态核验。
- Finder 显示文件已通过。普通文件拖入在修复 `63e808a` 后也完成真实验收：用户确认能从文件树打开内容，实际文件为 52 字节，与测试源的 SHA-256 完全一致，创建和修改时间均与原生导入提交对应。目标是用户当前会话选择的“测试”工作区。先前测试启动参数造成的 `NO_CONTEXT`、消息输入框附件和前端 `SecurityError` 均另行保留，未计作成功。收据 `artifacts/macos-port/live-finder-import-manual.json`；带子目录的整目录拖入也已通过，详见目录位置对照验收。
- 卸载失败恢复修复提交 `06799a8`。程序先暂存当前应用包及存在的上一版本，再撤销启动项和按需清除数据。不可变包导致暂存移动失败时，凭证尚未删除。暂存、撤销启动项或清除数据阶段失败时，程序尝试恢复已暂存的包，原位冲突时保留并报告副本。最终删除暂存目录在此恢复范围之外，已成功撤销的启动项和已删除的凭证与数据不会恢复。安装模块 11 项定向测试及独立复核通过，包含安装、回滚和卸载。完整 Rust 工作区 797 通过、0 失败、15 显式忽略。
- 原生透明补充排查：官方宿主确有原生“半透明侧边栏”设置，但它是持久、配置级的 macOS vibrancy，受失焦和大窗口强制不透明策略影响。未找到单窗口清透背景及崩溃后精确恢复接口，不能将此设置计为 Windows 透明插件等价通过。
- PowerPoint 在只读会话中能打开两页 PPTX；可写 `0600` 文件对照仍无法导出，保存命令无 PNG 产物，宏保护合同也未确认。用户明确允许改用 WPS。WPS 12.1.25895 已通过本地界面将旧版 PPT 导出为两张不同的 3840×2880 PNG，OCR 确认页序与内容，源文件哈希未变。收据位于 `artifacts/macos-port/office-probe/wps-legacy-ppt/receipt.json`。自动预览适配尚未通过：没有已验证的本地导出接口，AX 未提供文档完整路径，独立 PID 也未证明可隔离现有 WPS 配置，不能将手动导出等同于安全自动化。手动可用路径为 WPS「文件 → 输出为图片 → 逐页输出 → 所有页 → PNG」。
- 本机 Developer ID Application 身份数为 0，正式公证尚未执行。经用户授权已创建公开 fork 并运行 Windows/Mac CI；远端结果见文末，不能以本机回归代替 Windows 验证。

## 登录后真实窗口验收（2026-11-15）

用户手动登录独立测试实例后，生产布局资格检查通过，正式 Explorer 注入成功。原有日常客户端保持运行；文件与插件检查只使用私有测试工作区的合成文件。模型消息限于下述两次分别授权的新会话尝试，未读取同步历史会话正文。

- 真实原生文件桥接的 11 项交互通过：读取、编辑、Command-S 保存、文件监听、新建中文与字面反斜杠文件名、重命名、确认删除；文件刷新时保留未保存草稿和预览。
- Markdown 插件从真实市场下载成功；29 个锁定插件通过原生 SHA/尺寸校验导入私有缓存。后者属于离线导入，不能计作 29 次在线下载。
- 10 类预览分别完成真实文件读取、格式呈现、禁用、缓存重新启用。另验 Excel 表格和两页 PPTX，不能据此推断旧版 PPT 已通过。
- 13 个动态背景完成启停、代表性参数修改/重新打开后保留/还原、图层回收。12 个画布背景采样到非空且包含多种颜色的像素；Glow Horizon 检查 DOM 图层、可见度与参数。Pixel Sculpt 首次 700 ms 采样为空，等待初始化后采样通过，记录保留首次结果。
- 自定义背景通过合成 PNG 导入、解码、参数保存和删除该测试资源；界面透明度通过启停、参数同步与样式清理。这不等于原生 NSWindow 透明。
- Git 历史通过合成仓库的两条提交、文件详情、差异打开及刷新。
- 账号插件启用、官方资格/身份读取及随启动安装的原生桥接通过。签名应用副本使用同一私有测试配置，启动日志确认 enabled/qualified/ready=true；未改官方应用签名或资源。
- 修复 Mac 原生偏好持久化初始化条件后，现有浏览器设置通过生产桥接自动保存。第二次启动保留 19 项设置，10 类预览无需重开开关即正常呈现。没有手工写入启用设置文件。
- 启动动画使用合成 WebM 验证：文件导入、原生存储完整性、冷启动读取、首帧显示、实际播放、就绪退出通过。无视频时按约定跳过的结果另行保留。
- 用户授权的一条真实固定消息已创建全新只读测试会话并发送；首轮返回 HTTP 502，终态 failed、inProgress=false，未重复发送。本机代理缺少 localhost 例外，已用官方内置 CLI + 本地模拟代理复现并修复子进程绕过规则；仅修改该子进程的 localhost 路由，保留外网代理。用户再次授权后，在另一个新会话只发送一次相同消息，返回准确的 `Mac 验收通过`，终态 completed、inProgress=false；成功会话已显示在独立测试窗口。此项验证原生 manager/启动桥接请求响应，未假定多账号切换、自动接续或点击发送按钮已完整验收。
- 端口校验现支持官方签名 Computer Use 子进程继承的同一监听 socket；连续两次检查同 UID 持有者、socket 身份与受信祖先。24 项串行测试通过，真实注入通过。Darwin 无法据此证明跨 UID 的全局独占性。

收据：`artifacts/macos-port/live-files.json`、`live-plugin-download.json`、`live-plugin-offline-import.json`、`live-previews.json`、`live-office-extra.json`、`live-appearance.json`、`live-custom-git.json`、`post-login-qualification-summary.json`、`live-native-preference-migration.json`、`live-account-startup-observations.json`、`live-cold-restart.json`、`live-startup-video-playback.json`、`live-message-once.json`、`live-message-retest.json`。这些是范围明确的验收结果，不是完整功能等价声明。

## 此前构建与隔离验证（2026-11-15）

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
| 真实隔离宿主 CDP / 正式 UI | 登录后主要路径通过 | 官方布局资格、原生监听者校验、Explorer 注入及真实文件/插件交互通过 |
| 应用发现/CLI/资源定位 | 已实现、定向验证通过 | 签名/非标准 app 名/嵌入 CLI；真实 Beta 未验 |
| 进程/socket 校验与 attach 生命周期 | 已实现、真实 attach 通过 | 包括签名 CUA 子进程共享 listener；24 串行测试；跨 UID 全局独占不能证明 |
| 文件服务/路径/存储/Finder | 普通文件及目录拖入真实验收通过 | APFS、路径复制和草稿保留通过；Finder 显示、文件打开、嵌套文件、空文件与空目录通过；宿主对受保护目录的限制保持有效 |
| 账号/密钥库/系统菜单 | 真实资格、原生持久化、启动桥接和当前账号请求响应通过 | localhost 代理缺陷修复后，重新授权的一次消息返回正确结果；多账号切换/取消/续接仍未完整验收 |
| 原生窗口透明 | 缺少所需宿主接口 | 官方侧边栏 vibrancy 不满足单窗口清透与恢复合同；CSS 不替代原生结果 |
| 旧版 PPT 原生导出 | WPS 手动导出通过；自动预览未通过 | 两页 PNG 的页序/内容和源文件不变已核对；自动化文档身份与进程隔离仍需解决 |
| 安装/更新/登录启动/卸载 | 本地实现、安装模块 11 项定向测试及打包核心 10 项安装生命周期检查通过 | 包含不可变包拒绝、可逆的暂存恢复和运行进程保护；真实 GUI 卸载及注销后重新登录待验；在线 Mac 更新源待配置 |
| 正式签名/公证/发布 | 制品准备脚本已交付，正式分发待完成 | 需要 Developer ID、公证和独立发布通道，以及经批准的原生发布者信任和正式签名包生命周期验收 |
| Windows 回归 | 配置内远端自动化检查通过 | Windows Server 2025：886 项 Rust 通过、29 项显式忽略；协议、界面、恢复及窗口检查通过；官方 AppX 安装环境验收仍待完成，详见文末 |

## 插件逐项记录

本机 Chrome 157 无头夹具。29 个发布包及其资源均按锁重算 SHA-256；市场夹具通过 63 项断言。锁定包表示实际发布字节进入测试；源 Worker 使用仓库源文件；宿主与原生服务按各夹具模拟。真实宿主的主要路径结果见上节；以下列仍保留离线范围，不能用离线夹具替代真实宿主结果。

| 插件 | 版本 | 类别 | 离线实测与缺口 | 登录后真实窗口实测 |
| --- | --- | --- | --- | --- |
| glow-horizon | 1.0.1 | appearance | 锁定包浏览器热启停、离线复用、草稿保留；待逐项参数与冷重启 | 启停、代表性参数保存/还原、图层清理通过 |
| black-hole | 1.0.6 | appearance | 源 Worker 首帧/回收、WebGL 像素淡入；待锁定包完整参数/恢复 | 启停、代表性参数保存/还原、图层清理通过 |
| heavenly-cloud | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 | 启停、代表性参数保存/还原、图层清理通过 |
| aurora-ionosphere | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 | 启停、代表性参数保存/还原、图层清理通过 |
| milky-way | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 | 启停、代表性参数保存/还原、图层清理通过 |
| mountain | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 | 启停、代表性参数保存/还原、图层清理通过 |
| cloud-train | 1.0.1 | appearance | 源 Worker 首帧、活跃帧与回收；待锁定包启停参数 | 启停、代表性参数保存/还原、图层清理通过 |
| blinking-squares | 1.0.1 | appearance | 源 Worker 绘制与回收；锁定包启停/深浅/系统主题恢复 | 启停、代表性参数保存/还原、图层清理通过 |
| particle-image | 1.0.5 | appearance | 源 Worker 绘制与回收；锁定包软件 WebGL 旋转/设置测试 | 启停、代表性参数保存/还原、图层清理通过 |
| pixel-sculpt | 1.0.13 | appearance | 源 Worker 数据与绘制回收；真实 controller 与替代 GPU 生命周期；待锁定包完整编辑 | 启停、代表性参数保存/还原、图层清理通过 |
| custom-background | 1.0.13 | appearance | 锁定包主工作区/预览窗口样式与清理；共享 CSS 合同 | 合成 PNG 导入/解码、参数保存、移除通过 |
| surface-opacity | 1.0.14 | appearance | 锁定包完整离线夹具通过（32初始/6冷登录/6视频就绪及拖动、帧解码、窄屏等）；预览标签保持透明，原生菜单与预览背景独立 | 实际蒙版样式与参数/清理通过；非原生透明 |
| transparent-background | 1.0.0 | appearance | 锁定包 CSS 启停与模拟原生 RPC 通过；G3 原生透明阻塞 | 原生窗口透明仍阻塞 |
| git-history | 1.0.0 | developer-tools | 锁定包面板、合成提交、关面板/晚回复回收；待实际宿主文件和仓库联动 | 合成提交列表/详情/差异/刷新通过 |
| codex-startup-transition | 1.1.33 | appearance | 锁定包离线早启动、背景/视频、缺包/禁用跳过、设置及退出 | 视频导入/原生持久化、冷启动首帧/播放/就绪退出通过 |
| markdown-preview | 1.0.0 | file-preview | 锁定包 Markdown 渲染、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| csv-preview | 1.0.1 | file-preview | 锁定包 CSV 表格、坏引号警告、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| diagram-preview | 1.0.1 | file-preview | 锁定包 PlantUML、非法 Draw.io 错误、取消、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| image-preview | 1.0.0 | file-preview | 锁定包 PNG 解码、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| video-preview | 1.0.0 | file-preview | 锁定包 WebM 元数据、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| pdf-preview | 1.0.1 | file-preview | 锁定包 PDF 画布、错误/取消、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| audio-preview | 1.0.0 | file-preview | 锁定包 WAV 元数据、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| office-preview | 1.0.0 | file-preview | 锁定包 DOCX、Worker/WASM 资源、错误/取消、禁用/缓存重启；旧版 PPT 原生导出阻塞 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| notebook-preview | 1.0.1 | file-preview | 锁定包 Notebook 单元格、坏 JSON/取消、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| model-preview | 1.0.0 | file-preview | 锁定包 glTF WebGL、错误/取消、禁用/缓存重启 | 真实读取/渲染、禁用/缓存启用、冷重启恢复通过 |
| ascii-tiles | 1.0.8 | appearance | 发布 Worker 夹具18通过；异常后自动清理1失败（基线缺陷），手动禁用/启用恢复；直接WebGL交互和11项打开恢复通过 | 启停/代表性参数/像素通过；异常自动清理仍有基线失败 |
| shallow-ocean | 1.0.17 | appearance | 源 Worker 绘制/回收；锁定包呈现与回退检查；待真实宿主 | 启停、代表性参数保存/还原、图层清理通过 |
| multi-account | 1.0.110 | developer-tools | 锁定包市场下载/缓存；合成面板/路由/世代隔离；旧账号夹具22通过、共享终态轮询1失败（基线缺陷）；真实账号未验证 | 启用/资格/原生启动通道及当前账号消息复验通过；双账号/取消/续接待验 |
| spectral-drape | 1.0.2 | appearance | 锁定包 Worker 实时和暂停、30 控件、启停、缓存恢复 | 启停、代表性参数保存/还原、图层清理通过 |

日志在 `artifacts/macos-port/plugin-offline/`、`artifacts/macos-port/{background,preview}-packages.log`；所有文件预览的发布包均未携带可供审计的 esbuild metafile，主预览 shell 仍通过可选依赖隔离检查。

## 尚未关闭的行为与发布缺口

- G2：登录后正式注入、文件交互和插件主要路径已通过；全部参数组合、冷重启与性能长期运行尚未完整验收。
- G3：官方签名应用提供配置级侧边栏 vibrancy，但尚无满足单窗口清透、恢复及故障退出合同的接口；CSS 透明与实际 NSWindow 透明分别记录。
- G4：真实官方资格/身份读取、随启动安装的桥接、当前账号一次请求响应已通过；真实多账号切换、模型/额度、发送取消及上下文延续仍未完整完成。零回合新会话没有落盘归档，不能用它假定 native.prepare 已通过。
- 旧 `.ppt` 的 WPS 手动导出已通过，自动预览仍未通过。Finder 显示文件、普通文件和普通用户目录拖入通过；受保护目录仍按宿主规则拒绝。实际 Mac 卸载对话框及注销后登录项行为、已安装官方 AppX 的 Windows 宿主、Developer ID、公证、正式签名包生命周期及 Mac 更新源均待完整验收。
- ASCII 异常后的自动清理，以及切回官方身份后 legacy account 的共享回合终态轮询，各保留一项失败；相应生产文件与 Windows 固定基线一致，未修改锁定发布插件来隐藏这些缺陷。
- 后台进程清理覆盖本产品启动的继承进程组；主动另建 session/group 的后代不受该机制约束，不能称为 Windows Job Object 的完全等价隔离。
- Windows 预览版本号沿用基线的文件身份、长度和时间戳方案。同一文件被等长原地改写且元数据未变化时，不能保证发现内容变化；远端一次快速改写已暴露这一边界。版本转换测试改为显式推进时间戳以验证拒绝过期版本，没有宣称解决该基线限制。替换文件且保留时间戳的身份校验另有测试覆盖。
- 本地 API 代理绕过属于共享实现，也改善 Windows 路径；Windows 原有就绪探测仍保留，配置内 Windows 自动化回归已在公开 fork 通过。

## 消息链路修复后的构建与开发产物

- 核心修复提交：`923a54c`。完整 Rust 工作区 **793 通过、0 失败、15 显式忽略**；代理修复的 6 项定向测试、官方 CLI/App Server 的本地模拟代理回归通过。
- ARM64 完整构建、UI typecheck/build 通过；测试窗口控制器 18 项 Python 回归通过，包含控制器丢失、进程组保留、配置复用互斥、关闭证明及启动前持久记录失败。
- 新包 9 项独立检查通过：ad-hoc 签名、ZIP/DMG 完整性、ARM64/minimum macOS 13.0 声明、包内 CLI、独立临时中文路径安装、错误哈希拒绝并保留原包。实际 OS 运行是 macOS 27.2.1，未冒充 macOS 13 实机验收。
- 当时产物目录为 `artifacts/macos-arm64-proxy-fixed-20261115/`。
- 前两版目录保留历史证据；`macos-arm64-development-20261115` 和 `macos-arm64-login-acceptance-20261115` 不包含最终代理修复。
- `Code-Codex.app`、`Code-Codex-0.6.20-arm64.dmg`、`Code-Codex-0.6.20-arm64.zip`
- 该目录的 `verification.json` 保存该版开发包的哈希、签名、部署目标及真实临时安装检查。升级和回滚行为的前轮收据继续保留。
- 重复验证入口：`scripts/build-macos.sh --release`、`scripts/macos/test_package.py`、`scripts/macos/test_probe.py`、`TMPDIR=/private/tmp cargo test --workspace --locked -- --test-threads=1`。
- 本轮终态日志：`artifacts/macos-port/proxy-fixed-{build,workspace,package}.log`；旧日志与基线插件失败继续保留。普通 TMPDIR 指向符号链接的首次运行被路径保护拒绝；最终验证使用规范路径 `/private/tmp`，没有削弱生产路径校验。

## 后续缺口修复构建

- 核心源码 `06799a8`：完整 Rust 工作区 797 通过、0 失败、15 显式忽略；ARM64 核心、GUI 和 UI 构建通过。
- 该阶段开发包目录 `artifacts/macos-arm64-gap-closure-20261115/` 包含 ad-hoc 签名 app、ZIP、DMG 及 `verification.json`。签名、归档完整性、ARM64 和最低系统版本声明检查通过。该包的核心另通过 10 项真实临时安装生命周期检查，日志 `artifacts/macos-port/gap-closure-lifecycle.log`。
- 测试启动器修复 `b930ab7` 属于开发验收工具，不改变包内二进制。session05 已正常关闭；当前使用 session06 继续手动验收，日常客户端保持运行。
- 正式签名和公证脚本可准备并验证制品，真实 Apple 提交仍待条件具备。原生安装器尚未配置正式发布者信任，受管安装与升级、回滚、CLI 与图形卸载以及在 Finder 中显示已安装应用包均待用 Developer ID 制品验收。正式分发仍未完成。见 [macOS 正式分发说明](macos-release.md)。

## Finder 拖入修复后的当前开发包

- 包内源代码 `63e808a`，ARM64 核心、GUI、UI 类型检查和构建通过。最新目录为 `artifacts/macos-arm64-finder-fixed-20261115/`，包含 app、DMG、ZIP 和 `verification.json`。本轮没有生产 Rust 逻辑改动，前述 797 项 Rust 通过记录属于 `06799a8`，未在本轮重跑。
- 新 app 及 ZIP 解压后的 app 均通过签名、ARM64、最低系统声明及存储布局校验；DMG 完整性和归档哈希已核对。仍是 ad-hoc 开发包，未公证。
- 真实窗口在用户切换会话后，`explorer.context` 已返回 compatible=true。此次文件树拖入的失败来自 `FileSystemFileHandle.getFile()` 抛出的 `SecurityError`。修复优先读取 macOS 拖放事件提供的普通 File；目录缺少遍历接口时明确拒绝，避免把占位对象复制为空文件。
- 修复通过 11 项定向测试、真实构造 FileList 的浏览器复现及 5 项原生导入恢复场景。后者覆盖提交响应丢失、刷新失败、会话切换、提交拒绝和上传失败，全部结束后无开放导入会话。普通文件另完成了真实 Finder 拖入、原生提交、文件内容与哈希核对及用户打开确认；普通 Documents 子目录的整体拖入也通过了用户操作和落盘核验，包含嵌套内容、零字节文件及空目录。
- 发布准备脚本提交 `9adcf8b`，分发条件修正 `4ded3ee`。38 项 Python 广泛回归通过，最终标准标记的 5 项定向测试和分发状态的 15 项发布回归也通过。制品状态为 `notarized-artifacts`，机器记录保留 `unconfigured-native-release-trust` 分发阻塞；本机没有执行 Apple 提交。

## Finder 目录位置对照验收

- 第一次合成目录放在 `~/Library/Application Support/Code-Codex-Mac-Testing/` 下，宿主提示无法打开文件夹，因为其中含有系统文件；目录没有导入。该目录实际只有 3 个合成文件和 2 个子目录，无符号链接，未读取受保护的用户文件或绕过拒绝。
- 在普通 Documents 子目录中用相同已知字节重建夹具后，用户按正常授权流程拖入成功，并确认能打开内容。原生提交成功；根文件 24 字节、嵌套文件 26 字节、空文件 0 字节均匹配源 SHA-256，子目录和空目录均存在。收据 `artifacts/macos-port/live-finder-directory-import.json` 保留两次结果。
- 这一位置对照与 Chromium 对 macOS 应用数据目录及 `~/Library` 子目录的保护规则一致。上游源码是解释依据，未作为本机 Owl 二进制实现的逐行证明。此项无需修改生产导入代码，明确拒绝后的跨接口重试未实现。后续普通目录验收夹具应位于 Documents 的独立子目录，不放在应用数据或系统目录中。[Chromium 目录保护规则](https://chromium.googlesource.com/chromium/src/+/refs/heads/main/chrome/browser/file_system_access/chrome_file_system_access_permission_context.cc#604)

## Continuous regression coverage

- Both workflows now allow manual dispatch and pushes to `codex/macos-*`. The Windows native job runs the complete locked Rust workspace suite after its native build, retaining the existing recovery and UI integration checks. Its timeout is 45 minutes.
- Mac CI runs all packaging, notarization-state, isolated-session and process-cleanup Python regressions before restoring the temporary CI keychain, then exercises native install/upgrade/rollback against its packaged app. No Apple submission or real-account test is added.
- Local verification on 2026-11-15: all 38 Python tests passed; all 10 native installation lifecycle checks passed with the current Finder-fixed packaged core. Both workflow files parsed as YAML and passed whitespace validation. This does not certify a remote Actions run.
- Windows compilation uses Windows C# and resource tools in the existing build script; cross-compiling on this Mac does not replace the native job. With explicit user authorization, the branch is now pushed to the public fork [terryhe824/code-codex](https://github.com/terryhe824/code-codex/tree/codex/macos-full-parity).
- First remote Mac run at `a570e35` [passed](https://github.com/terryhe824/code-codex/actions/runs/43712551636): 797 Rust tests (15 explicitly ignored), 38 Python tests, 70 shared Node contracts and 10 native lifecycle checks. The ARM64 development app, ZIP and DMG were uploaded by CI; these remain ad-hoc signed and not notarized.
- First Windows contracts run failed while Chrome held a write lock on `DevToolsActivePort`. Commit `7c368c3` waits for a readable, complete and valid port within the existing startup deadline, with three focused regression tests. Local verification passed all 73 Node contracts and 172 browser surface checks. Its [Windows run](https://github.com/terryhe824/code-codex/actions/runs/43713047250) passed protocol and contracts jobs; the superseded native job was cancelled after reproducing the known path assertions. Its [Mac run](https://github.com/terryhe824/code-codex/actions/runs/43713046695) passed completely.
- The initial native Windows suite exposed three assertions comparing equivalent paths with different spellings: the runner's 8.3 temporary-directory alias versus its canonical name, and a standard-library verbatim prefix versus the normalized shell path. Commit `d629772` compares canonical targets while retaining absolute-path, leaf-label, stale-context and invalid-reveal checks. All 55 local bridge tests passed. The full Windows workspace invocation now collects all crate results with `--no-fail-fast`.
- The same commit's [Mac CI](https://github.com/terryhe824/code-codex/actions/runs/43713756766) passed on macOS 15.8.2 ARM64: 797 Rust tests (15 ignored), 38 Python tests, 73 shared Node contracts and 10 native lifecycle checks. Development artifacts were uploaded. This adds macOS 15 runner evidence, not macOS 13 runtime acceptance. Its [Windows CI](https://github.com/terryhe824/code-codex/actions/runs/43713756837) passed all 534 launcher-core tests, including the three corrected path assertions, but exposed further shortcut-fixture and media-version-timing failures in other targets.
- Commit `2442f2e` canonicalizes the native Unicode shortcut fixture and supplies synthetic icon PNGs through the existing test shell interface. Real COM shortcut inspection, creation and native atomic replacement remain in use; installed official AppX discovery/icon extraction are outside those transaction tests. The model-resource version-transition fixture explicitly advances mtime, making its intended metadata-version change deterministic. No production behavior or ignore list changed in that commit. Local verification: one targeted model-resource test and 46 portable shortcut tests passed. [Mac CI](https://github.com/terryhe824/code-codex/actions/runs/43715105137) passed completely. [Windows CI](https://github.com/terryhe824/code-codex/actions/runs/43715105079) passed the native transaction and model-resource checks; its sole remaining workspace failure was Shell parsing of an uninstalled official AppX target (HRESULT `0x80070002`).
- Commit `7c7e219` separates the installed-package Shell target check from the ordinary Unicode link round-trip. The latter and malformed-file checks remain enabled in CI. `installed_codex_packaged_target_roundtrip` is explicitly ignored by default because it requires the official Codex AppX package on a provisioned Windows host; its original assertions remain intact. This is a pending real-installation acceptance item, not a passing fixture result. On such a host, run `cargo test --release --locked -p code-codex --bin code-codex-shortcut native_shortcut::tests::installed_codex_packaged_target_roundtrip -- --ignored --exact`.
- Final verified source: `7c7e219826f6e465c1855887d2ba79e39a7e41b7`. [Windows CI passed](https://github.com/terryhe824/code-codex/actions/runs/43716529965) on Windows Server 2025 x64: complete Rust workspace 886 passed, 0 failed, 29 explicitly ignored; all three jobs passed. Additional native recovery checks passed: 90 conflict checks, real compiled transport/reset, 55 diagnostic-window checks in each of English and Chinese, 55 compiled launcher checks, and 59 native window recovery checks. Protocol, UI contracts and account handoff fixtures also passed. No real Codex process was installed, activated or closed.
- [Mac CI passed at the same source](https://github.com/terryhe824/code-codex/actions/runs/43716530046) on macOS 15.8.2 ARM64: 797 Rust passed, 0 failed, 15 explicitly ignored; 38 Python tests, 73 shared Node contracts and 10 native installation lifecycle checks passed. CI uploaded the ad-hoc development app, ZIP and DMG. Full logs and the local history receipt are retained under `artifacts/macos-port/`. These successful configured workflows do not close the real-account, installed-AppX, macOS 13, native transparency, automatic legacy-PPT or production-signing gaps above.

## 开发包继续验收（2026-11-15）

- 按用户选择继续开发包验收，Developer ID 和正式公证暂缓。新包目录 `artifacts/macos-arm64-development-accepted-20261115/`，源码 `3f0e92a`（相对双平台通过的 `7c7e219` 仅文档变化），完整 release/UI 构建通过。
- 原包、ZIP 解压包、只读挂载 DMG 中的应用均通过签名、ARM64、最低系统声明、CLI 版本及原生内容指纹核验。归档内应用指纹与原包一致；DMG 完整性通过。新包核心的 10 项临时安装生命周期检查全部通过，未注册或撤销当前用户登录项。
- 真实打包核心的卸载入口在受删除保护的临时副本上运行，先验证系统拒绝移动该副本，避免误选卸载时进入后续用户状态更改。约 60.2 秒后返回 `cancelled=true`、`uninstalled=false`；副本内容未变，原日常客户端和独立验收进程未变，临时副本已解除保护并清理。系统对话框不在可控应用清单中，未获取可见性或手动点击证据，因此只记录取消返回路径，其他卸载交互仍待验。收据 `artifacts/macos-port/development-uninstall-dialog-cancel.json`。
- 当前独立测试会话继续保留，无新增模型消息。安装说明已更新到新包，并改用实际支持的 `--command status|attach|detach|stop` 控制接口，移除过时的终端交互说明。
