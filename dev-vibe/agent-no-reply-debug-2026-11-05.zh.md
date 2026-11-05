# Desktop Agent 无回复修复与实测（2026-11-05）

## 源码与隔离范围

- Desktop：`work/hagency-desktop-local-20261028`，基线 `ea4963d13cf7a8fb212c5d2b387714cefc2ba165`。安装版内嵌源码 `b00459166744dab2ec8433da8176b1521f064e21`，两者仅发布文档不同。
- Server：独立 worktree `../.worktrees/hagency-server-agent-debug-20261105`，分支 `work/hagency-server-local-20261028`，源码 `7acbb25c9acb2ddf181b479ecef874662fb583d9`。
- Engine：从 Desktop 实际固定的 `19539818e927a2a2328eebba91d8736fab22c9c6` 建独立副本 `../.worktrees/hagency-desktop-engine-debug-20261105`，分支 `codex/recover-owner-direct-room`，本地修复提交 `dec4911b45acb6789be96e5a5d75bf8d357f30d1`。没有编辑 Cargo 缓存或 Client main。
- 使用本地 loopback 服务器、三个新 PostgreSQL 数据库和独立 Desktop 数据目录。私有配置、凭据、日志、原账号备份均留在忽略的 `.run/agent-no-reply-20261105`，不入 Git。

## 观察与本地实测

原账号 Agent 身份 active，但 owner-direct 日志为 creating / owner_direct_creation_outcome_unknown / room null。原聊天存在，原 hello 已送达；未见 Agent 入群。稳定别名的真实目录查询为 HTTP404。旧恢复代码只通过别名寻找未知创建结果，且禁止重复 createRoom，因此无法认领已产生但别名未落库的房间。

Pinned Palpo 的 createRoom 先写房间与 canonical_alias / initial_state，再等待邀请处理，最后写别名目录；这可以产生上述部分创建状态。尚不能仅靠历史日志确定原请求具体在哪一步失败或取消。

新本地测试账号经真实 OAuth 登录，创建 Local-Debug-Reply-1105、建立私聊、确认 Agent 入群、连接本机现有 Codex 账号并保存 GPT-6.1-Sol / low 配置后启动。真实消息 LOCAL_AGENT_REPLY_OK 得到该 Agent 同文回包。执行账本 settled，input4624 / output33 / cached_input3584 / reasoning_output23，reservation10000。回包后通过界面停止测试服务。

## 修复

- 未知创建 + 别名404时，只读扫描有限数量的 joined_rooms，以创建标记、所有者、Agent、canonical_alias、私密状态及两方成员验证唯一原房间。任一状态请求失败、多个候选或隐私/身份不符均保留失败，不重建 Agent 或房间，不手改运行记录。
- 服务 watcher 已完成发现但没有任何房间时，My Agents 显示 attention，避免永久显示 Starting。正常启动阶段仍显示 Starting。

## 验证与限制

- Desktop / Server 基线真实构建成功；本地 healthz / readyz 为200；实际 OAuth、创建、入群与模型回复通过。
- Engine owner-direct 测试7项通过，含未知创建候选、歧义、状态读取失败、越界/重复列表、身份隐私验证及原有持久化/索引测试。
- 本地补丁使用 `.run/agent-no-reply-20261105/engine-patch.toml` 显式 Cargo override，生产 Cargo.toml 的远端 pin 没有改动。
- 严格回归驱动运行后在 owner-source-pins 真实失败：本地补丁不属于已发布固定远端提交。此项不算通过；当前产物仅用于调试，不能作为正式发布资格。
- 全量 Desktop 测试（显式本地 engine override）：767通过、0失败、2项原有忽略。首次 --locked 调用因切换本地补丁后的锁文件需重新解析而实际失败；后续 --offline 完整测试通过，原日志保留。
- 原账号回归期间曾误启动两个相同数据目录开发实例，观察到401/invalid_grant及原生授权清理互斥失败；已关闭重复实例，并在再次打开 CUA 前确认只存在一个初始化完成的进程。单实例恢复后原账号授权正常，原 Agent 服务启动恢复继续。不能将该次调试并发造成的失败归咎于原线上服务或当作私聊修复成功。
- 单实例原账号实测：SDK正常刷新，My Agents 中原 chris-agent2 已从永久 Starting 改为 Needs attention。点击原 Agent 的 Set up / retry private chat 后，修复代码找回原房间，日志从 room null / creating 经过 created、pending，最后到 active / lastError null。服务器回读 owner-direct 绑定为 active，Agent 实际加入原房间。
- 在保留原 hello 的同一房间发送 `Please reply only ORIGINAL_AGENT_REPLY_OK. Do not use any tools.`，原 chris-agent2 真实回复 `ORIGINAL_AGENT_REPLY_OK`。原执行账本 settled，input7095 / output9 / cached_input6016，reservation10000。没有创建替代 Agent / 房间、手改状态、删除未知执行或增加原账号预算。
- `cargo --config ... check --locked --lib` 在配对本地调试锁文件下退出0。测试后已保留 `.run/agent-no-reply-20261105/Cargo.local-engine.lock` 并恢复仓库原始 Cargo.lock；原生产 pin 检查再次通过。之前本地补丁模式的严格发布门禁失败记录仍保留，不因此声称该补丁具备发布资格。
- 当前保留一个开发应用实例，使用原账号并可回复；隔离账号测试服务已停止。本地 Server 保留在 `http://127.0.0.1:19888`。安装版未覆盖，Server 未部署到原远端；代码未推送或发版。

## 复现本地构建

在 Desktop 根目录执行（先正常退出同一开发包实例，避免同数据目录双进程）：

```bash
cp Cargo.lock .run/agent-no-reply-20261105/Cargo.before-next-debug.lock
cp .run/agent-no-reply-20261105/Cargo.local-engine.lock Cargo.lock
cargo --config .run/agent-no-reply-20261105/engine-patch.toml build --locked --bin rinx
cp .run/agent-no-reply-20261105/Cargo.before-next-debug.lock Cargo.lock
RUST_LOG='rinx=debug,hagency=debug,info' bash packaging/run-macos.sh target/debug/rinx
```

以上使用明确的本地源码 override；默认未加 override 的 Cargo 构建仍使用生产固定远端引擎。调试启动器完全初始化后再由桌面工具选择现有应用；不要在其打包/启动间隙另开同一包。原始配置、完整日志、测试报告和服务器/账本回读均在忽略目录。

