# Desktop 0.5.0 发布

更新：2026-11-05。

## 2026-11-05 rc.2 发布

用户要求重新发布，包含最新的 Windows 工作和聊天历史的本地修复，并要求快速完成；由 Claude 执行。

- [v0.5.0-rc.2](https://github.com/hagency-org/hagency-desktop/releases/tag/v0.5.0-rc.2) 于 2026-11-05 06:16:32 UTC 公开为 prerelease，没有替换 stable latest。标签和全部二进制源码统一为 Desktop `b00459166744dab2ec8433da8176b1521f064e21`（`work/hagency-desktop-local-20261028` 顶端）。
- Client 固定为 `19539818e927a2a2328eebba91d8736fab22c9c6`：Codex 的 agent-startup-history 分支（加密房间分页读取历史、原始／继任／预算三种执行驱动接入、消息和密钥事件唤醒加五秒兜底），含 Claude 的历史硬化修复 94850cad。Codex 通过 PR #4（763b5a7e）合入的内容与 Claude 的直接合并逐字节一致。
- Makepad 固定为 `a0ad1839c61cecbd7e83251cfb7d50d7a725ee48`（OctoSense fork 分支 `fix/hagency-windows-platform-tests`）：即 Codex 的 Windows 系统浏览器分支 `codex/windows-system-browser-20261104` 顶端 7c6f6cc8 加一行测试修复——`platform/src/retained_instances.rs` 的 `sum()` 在 Windows 上因 WebView2 把 serde_json 带入依赖图而类型歧义，makepad-platform 的 lib 测试无法编译，release.yml 的 Windows 图形启动检查因此失败。Codex 若继续该 fork 分支，请基于 a0ad1839。
- 桌面侧还包含待批 Agent 权限请求的应用内提醒、房间角标、Agent 标签提示和原位置卡片（859adaca），以及 Windows 系统浏览器、分隔条命中范围和光标可见性（66515d20）。`windows-single-exe-*` 分支是测试启动器，未纳入。
- 发布由 release.yml 第四次 dispatch（run 41966327510，publish=true，macos-arm64-windows，hosting=true）自动打标签并发布。前三次失败：(1) 待批提醒三条文案未进翻译目录；(2) 门禁自 1827126e 起改在 macOS 15 机器运行，`apple-chat-fonts`／`apple-system-fonts` 失败——探针（临时 probe 分支，run 41964220918，已删除）证实 macOS 15.8.2 镜像没有 PingFang 字体资产，CoreText 把 PingFangSC-Regular 指向 PingFangUI.ttc，其中只有 `.PingFangUIText…-Default` 变量字体和 Medium 字重；现在 `article_makepad::apple_fonts::installed()` 为假时相关断言测试显式跳过并打印原因，目录错误用例改为不依赖系统字体；(3) 上述 Makepad 编译错误。
- 恰好三个安装包：`hagency-desktop-macos-arm64.dmg`（154,623,798 字节）、`hagency-desktop-windows-amd64.exe`（116,828,093）、`hagency-hosting-macos-arm64.dmg`（91,986,406），附两个 build report 和三个 SHA256SUMS。回归门禁在 macOS 15 机器上全部通过；两平台打包任务内的安装启动冒烟、Windows 图形启动检查、Hosting 生命周期和语言切换检查全部通过。rc.1 额外做过的独立 verify_only 验证和人工截图检查本次没有做；Mac 仍为 ad-hoc 签名，Windows 未签名。
- 本地证据在 `hagency-desktop-context-20261104/.rc2-*.log`（按 CI 原命令复现：桌面 766 单测、契约 10、字体 7、makepad-platform 3）。Codex 历史修复的 River 真机复测仍未做，仍待用户授权。macOS 打包任务在 GitHub 2 核机器上耗时约 2 小时；用户问能否自建，River 主机（M2 Pro 10 核，macOS 14.8.5，未装 Rust）可注册为 self-hosted runner 承担 macOS 任务，Linux 机器做不了 Windows 和 macOS 包。

## 2026-11-04 RC 发布

用户明确要求更新 Makepad 并发布 macOS / Windows RC，另提供带中英文切换的 resource daemon 控制应用，并排除 Intel Mac。本次发布完成，以下状态取代 10 月 16 日的暂停发布决定；原记录保留在下方。

- [v0.5.0-rc.1](https://github.com/hagency-org/hagency-desktop/releases/tag/v0.5.0-rc.1) 于 2026-11-04 16:56:52 UTC 公开为 prerelease，没有替换 stable latest。Release ID `433792626`。标签和全部二进制源码统一为 Desktop `1827126ede1bec7b83547b871b735625b6c83890`。
- Client 固定为 `a06e4cc204989bb36e666d59f6510ae7a0489a77`；Makepad 固定为 `ebec2e5c866eae3c791e2f24d59db543cad97a05`，包含 #132 的 Windows 焦点／次级窗口重绘修复，以及软件图形设备回退、静止窗口接收异步 shader 编译结果的修复。Makepad 源码在 OctoSense fork 的 `fix/hagency-windows-software-device` 分支公开。
- 恰好三个安装包：`hagency-desktop-macos-arm64.dmg`、`hagency-desktop-windows-amd64.exe`、`hagency-hosting-macos-arm64.dmg`。Hosting 包含 controller 和 daemon，支持跟随系统／English／简体中文，即时切换并跨重启保存。没有 Intel macOS 包，也不宣称支持 Windows 原生资源托管。
- Mac 安装包在隔离 Apple Silicon worktree、macOS 26.6.2 上构建，使用 pinned cargo-packager 0.10.1。Mac CI 未分配到运行器即因容量不足被取消，因此没有宣称整轮跨平台 CI 成功。Windows [job 126660061999](https://github.com/hagency-org/hagency-desktop/actions/runs/41854657931/job/126660061999) 的应用测试、图形设备启动／重建、打包、安装和真实渲染检查全部 success；所属 run 41854657931 总状态为 failure，不能把它写成整轮 green。
- Mac 760 个 Desktop 单测、10 个 app-contract 单测通过；完整回归 95/95 组、699 次 Rust 测试执行通过（与单测存在重叠，不是合并后的独立测试总数）。翻译检查 2804 条、1596 调用点、172 权限，缺失 0。隔离测试首次因脚本 VM 的 target 路径不匹配而停止，保留失败证据后修正构建路径并从首个未通过检查续跑，没有改动产品源码或跳过检查。
- 实际两个 Desktop 安装包均以干净 profile 启动，连续三帧登录页非空且资源加载正常；Windows 原生桌面截图与渲染截图一致。实际 Hosting DMG 中的 controller／daemon 哈希与构建记录一致，7 项生命周期检查、8 项语言切换／重启保存检查通过。六张实际截图已人工检查。
- Release 附有两个 build report、三个 verification report，以及每平台和总 SHA256SUMS；11 个公开附件的哈希和大小逐一匹配。匿名访问 Release 成功，标签确认为上述源码提交，三个安装包下载地址均返回 HTTP 200。
- Mac 为 ad-hoc 签名、未 notarize；Windows 未签名。发布说明已明确这些限制，保留既有依赖审计的开放事项，没有声称所有上游依赖告警已修复。

## 2026-10-16 历史记录

更新：2026-10-16。

- 用户确认发布目标为 hagency-desktop，要求版本 0.5.0，取代继承的 1.1.2；Cloud 不修改版本。
- hagency 依赖改为 https://github.com/chrislearn/hagency-client.git 的 main 分支；Cargo.lock 锁定 39e990c8d483d6bd29b4135742f486e7b83ad137，14 个引擎 crate 均来自 Git。删除 sibling checkout CI action 和全部调用，构建仍用 --locked。
- Windows cargo check --locked --lib 通过；i18n 缺失 0、主题检查通过。五平台测试及七安装包由 GitHub release.yml 的 publish=true 手动运行验收，全部成功后自动打标签和发布，不能把 dispatch 当成发布成功。
- 发布说明在 packaging/release-notes.md，说明功能、版本回退及 Agent 加密 Room、签名和移动端限制。发布显式 --latest，保证低于 1.1.2 的 0.5.0 仍成为下载入口。
- Cloud 新增 docs-tenant/{en,zh}/agents.md 和 downloads.md，包含七个官方 latest/download 安装包链接；其发布进度在 Cloud dev-vibe/memory/desktop-downloads-and-agents.md 记录。

首轮安装包运行38832134391的macOS ARM64编译和459单测通过，但打包app失败：cargo-packager0.10.1 No matching IconType。此前品牌改动将普通1024 PNG加入icons但未列现成ICNS；上游create_icns_file对1024@1x转换报错，显式ICNS会优先原样复制。现把resources/icon.icns列入Cargo packager icons（Linux仅取PNG、Windows优先ICO，不受影响），新增tools/check-release-icons.py及CI／发布前置校验。Windows locked库编译与真实图标输入检查通过。必须用修复提交重新完整构建后发布，不能重用旧提交混合附件。

最新状态（覆盖此前发布计划）：因发布审计未达标，38832134391、38837925791均取消，没有v0.5.0标签或公开Release。Desktop主lock的Rustls0.23.46、webpki0.103.15、AWS-LC1.18.2/sys0.46.0、Quinn0.11.18兼容安全补丁将漏洞匹配24降到5，仍21其他警告；Windows locked check与456单测通过（2忽略）。随包Octos fe08独立Cargo.lock仍36漏洞匹配+14其他警告，Desktop补丁不会传递过去。实际PDF抽取及RSS属性解析存在需修补路径；不能把所有完整锁文件记录都当成已证实可利用。报告与机器证据见../release-0.5.0/assessment.zh.md。已询问用户是否扩大范围修复上游后再发，未获答复前不再启动正式发布；Cloud上线同样待其独立确认。

最终验证：7ad1b8dca67222bd8312952799b3dc397e12062a的主CI38842267586与五平台构建38842267688均success（远端Mac461pass/2ignore、本地Win456pass/2ignore），翻译缺失0、主题与图标前置检查通过。正式发布仍暂缓；未获新的上游升级范围答复，不自动开启第三轮发布或扩改上游。独立Octos锁已与fe08的Git对象逐字节比对，SHA256 cfed26d0a98f515574607f6015a9547caccace48df43f510c2df38063fe45e64。
