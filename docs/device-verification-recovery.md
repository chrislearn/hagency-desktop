# 设备验证与密钥恢复

在 Settings → Account 的 Encryption and recovery 中查看设备与恢复状态。
设备验证确认设备身份；读取加密历史还需要对应的房间密钥。

## 首次设置

1. 在尚未配置恢复的账号上选择 Set up recovery。
2. 将生成的恢复密钥保存到私有文件，并另存一份到设备之外。
3. 保存成功后，选择 I saved my key — enable recovery。
4. 可用的房间密钥随后进入备份；Back up pending room keys 可等待当前已知密钥上传。

密钥文件使用操作系统的私有文件权限，拒绝覆盖已有文件和读取符号链接。
设置进度只记录公开的账号、设备、密钥标识及校验元数据，不将恢复密钥写入进度文件。
已经开始发表的设置保留原意图；中断后使用 Load saved key to resume 继续原设置。

## 恢复已有账号

输入已有恢复密钥，或通过 Restore from key file 选择密钥文件。
恢复会导入已有身份秘密及备份密钥，并下载服务器中可用的房间密钥；失败会显示错误，
可以用原密钥重试。流程保留既有身份、Secret Storage 和备份，不自动重置它们。

状态分别表示设备是否已验证、当前设备是否持有身份密钥，以及是否已连接房间密钥备份。
“账号已配置恢复”不能作为当前设备已恢复密钥的证明。恢复完成后关闭 Settings，
原来打开的聊天会处理积压的解密更新，无需重启或重新选择 Room。

验证弹窗及其异步结果绑定到账号会话、对端用户和 SAS flow。切换账号或重新登录后，
旧结果和旧窗口输入失效；关闭账号任务域时清除内存中保管的恢复密钥。

## SDK 与来源

从 Client PR #38（4fc7c3caa）和 #43（091931f21）迁入，适配 Desktop 当前的样式、
客户端槽修订号、设置导航及 crossbeam 时间线通道。

保持 Matrix SDK 版本 0.19.1，将整个 SDK 家族统一固定到
`chrislearn/matrix-rust-sdk@c4051424fe9ac2bea56e5e00c900d6c87294c0b7`。
Desktop 与 `crates/hagency-client` 独立工作区及锁文件使用同一来源。
其中包含：

- 5f76abef7：预先保存恢复密钥，启用时发表同一密钥；调试输出脱敏。
- a54afad0d：导入秘密时释放 Olm 读锁，再等待网络 IO。
- 95115b95b：恢复 SQLite 身份后的设备签名验证。
- c4051424f：OneShot 历史密钥下载失败可见，已启用备份仍可重试下载。

依赖图只使用该提交的 SDK 家族，没有混入 registry SDK。静态核对确认 SQL 迁移文件，
以及 state/crypto/event-cache 的 run_migrations 与 registry 0.19.1 一致。
其他依赖版本及 Makepad pin 保持原样。

## 验证边界

回归覆盖私有密钥文件、原密钥设置进度、状态区分、过期结果、精确验证会话、
以及返回被设置页覆盖的原生聊天后刷新缓存。固定 SDK 的真实 HTTP 集成回归另外覆盖
原密钥发表、历史下载失败与重试、以及秘密导入期间的 Olm 锁释放。
测试使用隔离资料目录和合成密钥，没有操作用户账号、生产备份或系统凭据。
新 Desktop 分支的人工生产登录与跨平台实机恢复仍需单独验收。

### SDK 定向复验

在上述固定提交的独立 SDK 检出中，使用 `cargo test --locked -p matrix-sdk --features testing`：

| 目标与筛选器 | 通过数量 |
| --- | ---: |
| `--test integration test_one_shot_recovery_reports_download_failure_and_retries_enabled_backup` | 1 |
| `--test integration test_prepared_secret_store_uses_the_saved_key_and_redacts_debug` | 1 |
| `--test integration encryption::recovery::test_recovery_setup` | 2 |
| `--test integration encryption::backups::test_enable_from_secret_storage` | 8 |
| `--lib importing_secrets_does_not_block_olm_recreation_during_network_io` | 1 |
| `--lib restored_sqlite_identity_can_sign_its_own_device` | 1 |

这些用例实际执行 SDK、HTTP 模拟服务、密钥导入及 SQLite；均使用 SDK 的合成测试资料。
Desktop 的完整库、预算/聊天门禁、翻译检查及二进制构建另行执行。
