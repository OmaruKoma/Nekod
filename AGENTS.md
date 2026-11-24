# Nekod — Agent Handoff & Current Progress Guide (`AGENTS.md`)

> **用途说明 / Purpose:** 本文档用于向新接入的 Coding Agent / 协作者同步 Nekod 仓库的**实时开发进度、未提交工作区变更、伴生桥接版本、关键工程约束（踩坑经验）及待办事项**。开始改代码前请务必先读完本文档。  
> **最后更新时间 / Last Updated:** `2026-11-24`（`v1.0.0` 正式版准备，基于分支 `main`，`HEAD = 660d707` + 当前工作区增量）

---

## 1. 必读文档顺序 / Mandatory Reading Order

1. [`.agents/rules/nekod-master-prompt.md`](.agents/rules/nekod-master-prompt.md) — 全局铁律（6 层架构、MIT 协议、U11 canonical slug、安全红线、测试与汇报规范）
2. [`ARCHITECTURE.md`](ARCHITECTURE.md) 与 [`TECHNICAL_DECISIONS.md`](TECHNICAL_DECISIONS.md) — 模块边界、`ADR-001`–`ADR-020`
3. [`ROADMAP.md`](ROADMAP.md) 与 [`docs/runtime-engine-ecosystem.md`](docs/runtime-engine-ecosystem.md) — `P0–P5` 及 `V1.1 / V2` 多引擎架构规划
4. [`docs/v1.1-roadmap.md`](docs/v1.1-roadmap.md) — 玩家端桌面 UX 与生态完善路线

---

## 2. 当前基线与 Git 工作区快照 / Live Baseline & Working Tree

- **当前分支与基线提交：** `main` @ `660d707`（`feat(unity-bridge): upgrade bridge to v0.4.0 with persistent pins, target lock, and damage aura`）
- **当前未提交增量规模：** 当前工作区包含尚未 `git commit` 的阶段性成果（涵盖 L1–L6、UE4SS `v0.7.3`、Unity `v0.5.5` 及多个示例 `.nmod` 包，请勿随意回退或覆盖）：
  - **L1 桌面端（Tauri 2 + React，正式版 `v1.0.0`）：**
    - `apps/desktop/src-tauri/`：`bridges.rs`、`lib.rs`、`overlay.rs`、`tauri.conf.json`、`icons/`（开启 `bundle` NSIS 安装包 + 无边框自绘顶栏 + 系统托盘常驻 + 官方 `Nekod-Mods` 源自动订阅 + `.nmod` 导出与全源商店查询 + 自定义应用图标）
    - `apps/desktop/src/`：采用 Deep Emerald Obsidian 深邃墨绿极客暗黑风（`#0B1B16` 侧栏 / `#0B0E0D` 主画布 / `#111514` 卡片 / `1px border-white/[0.08]` 描边 / `#38B788` 薄荷翡翠绿点缀 / `1.5px` 图标线宽 / `#7D8C86` 次要文字），已移除海报墙，常驻两栏工作台并内置 `i18n/index.tsx`（`zh-CN` / `en-US` / `ja-JP`），包含 `TitleBar.tsx`、`GameSidebar.tsx`、`GameWorkbench.tsx`、`ModStoreView.tsx`、`ImportCenterView.tsx`、`SettingsView.tsx`、`LogsView.tsx`、`Modals.tsx`、`HudOverlay.tsx` 与 [`styles.css`](apps/desktop/src/styles.css)
  - **L2 核心 & 管理器 & 仓库：** `crates/game-detection/src/engine.rs`、`crates/trainer-manager/src/lib.rs`、`crates/nekod-core/src/lib.rs`、`crates/registry-client/src/lib.rs`（支持 `nmod_v1` `mod-manifest.json` 与同级 `game-manifest.json` 校验下载与自动注册）
  - **L3 运行时 & L4 API：** `sdk/runtime-api/src/lib.rs`、`crates/trainer-runtime/src/`（`ue4ss_bridge.rs`、`unity_bridge.rs`、`bridge_ipc.rs`、`frida_gum.rs`、`lib.rs` 等）
  - **L5 校验器：** `tools/trainer-validator/src/lib.rs`
  - **L6 MCP 工具：** `crates/nekod-mcp/src/lib.rs`、`server.rs`、`tests/stdio_ipc.rs`
  - **伴生脚本插件：** `plugins/ue4ss-nekod-bridge/Scripts/main.lua`、`plugins/unity-nekod-bridge/`
  - **独立 `.nmod` 仓库拆分：** 5 个实机验证游戏 `.nmod` 包（`seed-of-the-dead-sh`、`aidealrays`、`aidealrays-playable-lily`、`sakura-segment-zh-cn`、`gun-soul-girl-2`）已迁移至独立仓库目录 [`d:\codespace\Nekod-Mods`](file:///d:/codespace/Nekod-Mods)，主仓 `examples/` 仅保留 3 个最小通用脚手架测试样例（`sample-singleplayer-rpg`、`unity-standalone-starter`、`unity-explorer-mcp-starter`）。
- **最近一次全量构建与测试状态：** **全部通过**（`dotnet build` Mono & IL2CPP、`npm --prefix apps/desktop run build`、`cargo fmt --all -- --check`、`cargo check --workspace`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 共 **150 passed, 0 failed, 0 ignored**）。

---

## 3. 各层核心模块与当前版本清单 / Current Component Status

### 3.1 核心平台里程碑
- **`P0–P5`（基础架构、Win32/Linux 运行时、仓库客户端、MCP/Skills、WASM 沙箱宿主）：** 已完成并持续由单测保护。
- **`V1.1-S1` & `V2-M1`–`V2-M5`（`.CT` 静态转换、`ModManifest v2 / nmod_v1`、`DeploymentLedger`、多引擎适配器、`Dependency Manager`、`AI Mod Forge` 管线）：** 已完成并持续围绕真实 Unity / Unreal 单机游戏实战打磨。

### 3.2 内置伴生桥接目录（[`apps/desktop/src-tauri/src/bridges.rs`](apps/desktop/src-tauri/src/bridges.rs)）

| Bridge ID | 适用引擎 | 当前版本 | 源码位置 | 核心能力摘要 |
| :--- | :--- | :--- | :--- | :--- |
| `nekod-ue4ss-bridge` | Unreal Engine 4/5 (`RE-UE4SS`) | **`0.7.3`** | [`plugins/ue4ss-nekod-bridge/Scripts/main.lua`](plugins/ue4ss-nekod-bridge/Scripts/main.lua) | 单线程 `LoopInGameThreadAfterFrames` 调度、纯 Lua File-IPC/TCP、只读反射探测、运行期 UMG 汉化+字体回退、粘性属性锁定、锁脖自瞄（最近怪物+平滑限速+零视差）、极速自动开火、世界空间解耦第三人称大陀螺、原生 `ApplyDamage` 杀戮光环、全道具补齐、`BP_Goal` 一键通关 |
| `nekod-unity-bridge-mono` | Unity Mono (`BepInEx 5`) | **`0.5.5`** | [`plugins/unity-nekod-bridge/BridgeServer.cs`](plugins/unity-nekod-bridge/BridgeServer.cs) | 零 UnityExplorer 依赖，自动发现 PID 与 Token、四元数无万向锁 `LateUpdate` 骨骼大陀螺+场景守卫自动重绑、通用 `DamageAura` 参数与 `dead_method`/`exclude_bool_field` 过滤、字段持续锁定与 `CopyField`、`SetGameObjectActive`（含隐藏对象）、Harmony 方法挂钩写字段/引用复制与 `clone_under_owner` 子对象克隆及 `PlayableDirector` 通用轨道/`ExposedReference` 与 Cinemachine `Follow`/`LookAt`/`TargetGroup` 自动重绑/补齐组件（`pin_method_hook` / `unpin_method_hook`）、受控包内贴图材质替换与回滚、只读反射探测（`list_members`/`find_instances`/`dump_hierarchy`） |
| `nekod-unity-bridge-il2cpp` | Unity IL2CPP (`BepInEx 6`) | **`0.5.5`** | [`plugins/unity-nekod-bridge/BridgeServer.cs`](plugins/unity-nekod-bridge/BridgeServer.cs) | 同上（IL2CPP 分支，含安全场景遍历、Harmony Mono/IL2CPP 通用方法挂钩、异步生命周期修复与 `Il2CppStructArray` 纹理加载） |
| `nekod-reframework-bridge` | RE Engine | `0.1.0` | [`plugins/reframework-nekod-bridge/reframework/autorun/nekod_bridge.lua`](plugins/reframework-nekod-bridge/reframework/autorun/nekod_bridge.lua) | 基础脚本桥接 |
| `nekod-godot-bridge` | Godot | `0.1.0` | [`plugins/godot-nekod-bridge/nekod_bridge.gd`](plugins/godot-nekod-bridge/nekod_bridge.gd) | 基础脚本桥接 |

### 3.3 `ue4ss_bridge_v1` 与 `unity_bridge_v1` 协议能力
- **`ue4ss_bridge_v1`**（[`crates/trainer-runtime/src/ue4ss_bridge.rs`](crates/trainer-runtime/src/ue4ss_bridge.rs)）：支持 **13 种** 声明式绑定：`CheatManagerCommand`、`CheatManagerFloat`、`ToggleUProperty`、`SetUPropertyScalar`、`AddUProperty`、`TargetLock`、`AutoFire`、`SpinBot`、`DamageAura`、`CallUFunction`、`UiTextLocalization`、`FillItems`、`TriggerGoal`。
- **`unity_bridge_v1`**（[`crates/trainer-runtime/src/unity_bridge.rs`](crates/trainer-runtime/src/unity_bridge.rs)）：支持 **10 种** 声明式绑定：`ToggleField`、`SetField`、`InvokeMethod`（含 `args` 传参）、`TargetLock`、`DamageAura`（含通用 `call` 参数占位符与传统快捷种）、`SpinBot`（支持 `LateUpdate` 骨骼自转与角度自动复原）、`SetGameObjectActive`（支持隐藏对象与前缀/名称数组）、`CopyField`、`ReplaceTextureMaterial`（经 [`StartSessionRequest`](sdk/runtime-api/src/lib.rs) 的 `verified_package_assets` 强制使用包内 SHA-256 验签资产并支持 `restore_texture_material` 回滚）、`HookSetField`（借助 BepInEx 内置 `0Harmony.dll` 在目标方法前置/后置执行字面量写入、跨对象引用复制或 `clone_under_owner` 子对象克隆与 `PlayableDirector` 轨道/`ExposedReference`/Cinemachine 重绑，或缺失组件补齐并初始化空 `List<T>`，支持自动回滚）。

### 3.4 桌面端新增能力（`L1`: `apps/desktop/`）
- **UE4SS 自动化部署与快捷脚手架：** 自动识别 `<Game>/<Project>/Binaries/Win64/ue4ss/`，支持 RE-UE4SS ZIP 校验解压入 `DeploymentLedger`、一键部署/升级 `NekodBridge`、`quick_setup_ue4ss_starter`。
- **RGB 游戏内浮窗 HUD（`hud-overlay`）：** 新增 [`apps/desktop/src-tauri/src/overlay.rs`](apps/desktop/src-tauri/src/overlay.rs) 与 [`apps/desktop/src/HudOverlay.tsx`](apps/desktop/src/HudOverlay.tsx)，以独立点击穿透透明窗口在游戏画面右上角实时展示当前激活的英文模块标签列表，并在 [`App.tsx`](apps/desktop/src/App.tsx) 工具栏提供开关与状态同步。

### 3.5 可选开发者 MCP 工具（`L6`: `crates/nekod-mcp/`）
- `PLANNED_MCP_TOOLS`（[`crates/nekod-mcp/src/lib.rs`](crates/nekod-mcp/src/lib.rs)）已实现 **24 个** JSON-RPC 2.0 工具，读写严格隔离并强制显式授权校验。
- `probe_unity_bridge` 只读白名单已覆盖 `ping`、`list_types`、`list_members`、`find_instances`、`dump_hierarchy`、`get_field`；与 `probe_ue4ss_bridge`、`inspect_unity_assemblies`、`forge_nmod_package`、`pack_nmod_archive`、`verify_forged_mod_package` 共同构成 Mod 分析—生成—校验闭环。

### 3.6 仓内最小示例与独立 `.nmod` 存储仓库
- **主仓库 (`examples/`)：** 保留 3 个平台级/单测用最小示例包：`sample-singleplayer-rpg`（`declarative_patch_v1`）、`unity-standalone-starter`（`nmod_v1` + `unity_bridge_v1`）、`unity-explorer-mcp-starter`（`nmod_v1` + `unity_bridge_v1`）。
- **独立 Mod 仓库 ([`d:\codespace\Nekod-Mods`](file:///d:/codespace/Nekod-Mods))：** 托管真实游戏 `.nmod` 包、`registry.json`（由 `scripts/generate_registry.py` 生成与校验）及 GitHub Actions 自动校验/打包工作流，客户端默认通过 `org.nekod.official-mods` 订阅：
  - `mods/seed-of-the-dead-sweet-home/seed-of-the-dead-sh/` (`v1.3.3`, 26 项, `ue4ss_bridge_v1`)
  - `mods/aidealrays/aidealrays/` (`v1.0.1`, 18 项, `unity_bridge_v1`)
  - `mods/aidealrays/aidealrays-playable-lily/` (`v1.0.5`, 14 项, `unity_bridge_v1`)
  - `mods/sakura-segment/sakura-segment-zh-cn/` (`v1.0.0`, 3 项, `ue4ss_bridge_v1`)
  - `mods/gun-soul-girl-2/gun-soul-girl-2/` (`v1.2.0`, 23 项, `unity_bridge_v1`)

---

## 4. 关键工程约束与实战踩坑记录 / Critical Invariants & Lessons Learned

后续修改 **RE-UE4SS 伴生桥接 / 自瞄 / 大陀螺 / 清单版本** 时，请严格遵守以下已用实机崩溃与试错换来的约束：

1. **RE-UE4SS 必须单线程帧泵调度（严禁并发 `LoopAsync` 修改 UObject）：**
   - [`main.lua`](plugins/ue4ss-nekod-bridge/Scripts/main.lua#L3619-L3693) 必须保持 **UTF-8 无 BOM**，并通过 `LoopInGameThreadAfterFrames(1, master_game_thread_tick)` 在游戏主线程单入口串联调度（仅当不可用时退回保护性 `LoopAsync_Guarded`）。
   - 执行顺序固定为：`poll_ipc` -> `tick_target_locks` -> `tick_spinbot` -> `tick_auto_fire` -> `tick_damage_auras` -> `sweep_active_persists`（`0.033s` / ~30Hz，压制游戏 `UpdateMoveSpeed` 减速）-> `sweep_live_text_blocks`。
2. **RE-UE4SS Lua 5.4.7 反射坑：**
   - **UDATA 相等判断：** UE4SS 返回的 `RemoteObject` 包装器不能直接用 `==` / `~=` 比较是否指向同一 `UObject`，必须用 [`same_uobject`](plugins/ue4ss-nekod-bridge/Scripts/main.lua#L1956) 对比 `GetAddress()`。
   - **Blueprint `OUT` 参数与空对象：** UE 蓝图函数的 `OUT` 参数在 UE4SS Lua 中需传空表 `{}`（例如 `K2_SetWorldLocationAndRotation(..., false, {}, true)`、`IsExecReload({})`）；不可传入 `nil` 给需要 `UObject*` 的槽位，请使用现有 `null_uobject()`。
   - **禁止原地修改引擎返回的 `FVector` userdata：** 返回给瞄准点的坐标必须新建只读 Lua 表 `{ X = ..., Y = ..., Z = ... }`。
   - **敌方对象过滤：** 实机中 `FindAllOf` 会扫出 `_GEN_VARIABLE` / `SKEL_` / `REINST_`（如 `ChildActor_GEN_VARIABLE_...`）或尚未 `BeginPlay` 的 Actor，需保持 [`is_archetype_name`](plugins/ue4ss-nekod-bridge/Scripts/main.lua#L421) 与 `HasActorBegunPlay() ~= false` 校验。
3. **锁敌与大陀螺稳定数学约束：**
   - **禁止混用 `cam_rot` 与 `ctrl_rot`：** `PlayerCameraManager:GetCameraRotation()` 相比 `pc:GetControlRotation()` 存在 1 帧延迟，用 `cam_rot` 算误差再加回 `ctrl_rot` 会构成正反馈震荡甩镜；角度差必须严格基于 `base_rot`（即 `ctrl_rot`）计算并受 `max_step = dt * 420°/s` 限速平滑。
   - **SpinBot 世界空间解耦：** `BP_PL_Player` 的组件层级为 `FPS_Mesh -> TPS_Base -> SpringArm -> TPSCamera`（其中 `FPS_MeshRoot` 带 `-90°` Yaw）。`SpringArm` 必须通过 `SetAbsolute(true, true, false)` + `K2_SetWorldLocationAndRotation` 脱离父节点旋转影响，并固定 `TargetOffset = (0, 0, 16)`、`SocketOffset = (0, 0, 0)`，与 [`get_spinbot_aim_origin`](plugins/ue4ss-nekod-bridge/Scripts/main.lua#L1530)（`actor_loc.Z + 80.0`）保持零视差与零轨道甩镜；关闭时需先恢复 `SetAbsolute` 再恢复原父组件与相对坐标。
   - **重扫与防抖：** `select_best_enemy_target` 仅在真正执行全量扫描时刷新 `scanned_at`（`0.05s`），`nearest` 模式下需新目标距离比当前锁定目标近至少 `80.0cm` 才切换，防止临界抖动。
4. **版本号 & 校验值联动更新清单：**
   若升级 `nekod-ue4ss-bridge` 或修改 `Nekod-Mods` 中的包清单，必须**同时检查并同步**：
   - [`plugins/ue4ss-nekod-bridge/Scripts/main.lua`](plugins/ue4ss-nekod-bridge/Scripts/main.lua)（头部注释 + `BRIDGE_VERSION`）
   - [`apps/desktop/src-tauri/src/bridges.rs`](apps/desktop/src-tauri/src/bridges.rs)（`BUILTIN_BRIDGES` 对应条目的 `version`）
   - [`crates/trainer-runtime/src/ue4ss_bridge.rs`](crates/trainer-runtime/src/ue4ss_bridge.rs)（过旧桥接升级提示文案 + 对应单测版本号与字段断言）
   - [`apps/desktop/src/types/ui.ts`](apps/desktop/src/types/ui.ts)（`formatRuntimeError` 版本检查列表与升级提示 + `OVERLAY_FEATURE_LABELS`）
   - 对应 `.nmod` 包的 `mod-manifest.json`（`version`、`version_req`、`runtime_version`、feature 列表与脚本 1:1 对齐，以及 `artifacts[*].sha256` 与 `size_bytes`），并在 `Nekod-Mods` 根目录执行 `python scripts/generate_registry.py` 刷新 `registry.json`
   - [`tools/trainer-validator/src/lib.rs`](tools/trainer-validator/src/lib.rs)（主仓最小包测试断言）

---

## 5. 不可逾越的安全与项目边界 / Non-Negotiable Rules

1. **禁止直接篡改外部游戏安装目录：** 绝对不要直接写入用户本机游戏目录（如 `D:\F95 Game\...`）中的任何文件；所有伴生脚本/Loader 部署必须由桌面端经 `DeploymentLedger` 托管，便于玩家侧检测外部改动与干净回滚。
2. **玩家端零 AI / 零 MCP：** `apps/desktop/` 严禁引入 `nekod-mcp`、AI API Key 或开发者底层内存调试面板；所有高危操作必须通过 `nekod-core` 与 `permission-manager` 默认拒绝（Default-Deny）授权与 `(trainer_id, version, manifest_sha256)` 哈希绑定。
3. **严格单机边界：** 禁止任何反作弊绕过/隐身注入、禁止自动提权（遇到权限不足向玩家提示非管理员启动游戏）。
4. **Git 规范：** 未经用户明确指示，不要自动执行 `git commit` 或 `git push`。

---

## 6. 当前待办与下一步建议 / Pending Work & Next Handoff Tasks

1. **推送 GitHub 正式版与独立仓库：**
   - 在用户确认后：
     - 将 `d:\codespace\Nekod-Mods` 初始化/推送到 `https://github.com/OmaruKoma/Nekod-Mods`（`main` 分支，保证 `raw.githubusercontent.com/OmaruKoma/Nekod-Mods/main/registry.json` 在线可拉取）。
     - 按功能模块整理提交 `d:\codespace\Nekod` 的 `v1.0.0` 变更，打 `v1.0.0` tag 触发 [`.github/workflows/release.yml`](.github/workflows/release.yml) 发布 Windows NSIS 安装包与便携版压缩包。
2. **后续迭代方向：**
   - 桌面端 `.nmod` 商店增补封面图预览与分类筛选；
   - 补充更多 UE4/UE5 与 Unity 单机参考 `.nmod` 包到 `Nekod-Mods` 并完善 `CommunityVerified` 自动化签名流程。

---

## 7. 必跑验证命令 / Mandatory Verification Gates

每次完成有意义的代码或清单元数据修改后，必须执行并核实实际输出（全部退出码为 `0` 且无跳过/无告警）：

```powershell
# 1. 前端 TypeScript 类型检查与 Vite 构建
npm --prefix d:\codespace\Nekod\apps\desktop run build

# 2. Rust 格式、工作区编译、Clippy（零警告）与全量测试
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3. 独立 Nekod-Mods 仓库生成检查与清单/包校验
python d:\codespace\Nekod-Mods\scripts\generate_registry.py --check
cargo run -p nekod-validator -- registry d:\codespace\Nekod-Mods\registry.json
```
