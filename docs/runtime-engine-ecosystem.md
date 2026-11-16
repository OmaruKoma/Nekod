# Nekod 多引擎 Mod 平台与 AI 生产生态架构设计

> **核心定位升级：**
> **Nekod — Modding games, together.**  
> **Open-source Community Mod Platform — Discover, create, manage, and share game modifications.**
>
> Nekod 不止于单一的 Trainer 启动器或 WeMod 替代品，而是把分散在 GitHub、网盘、论坛、`.CT` 表格、各类 Mod Loader 与游戏专用脚本生态里的修改能力，统一收敛到同一套开放协议与安全边界下；同时通过 **Player Ecosystem** 与 **Developer / AI Mod Forge** 双生态闭环，既让普通玩家零门槛开箱即用，也让开发者与 AI Agent 能分析游戏、生成 Mod 并经实测验证后发布到社区。

---

## 1. 核心架构判断：多引擎 Mod Runtime 平台

Nekod 保留原有的 **L1–L6 六层逻辑架构**，但 **L3 Runtime** 不再局限于单一执行机制，而是升级为 **Unified Runtime Interface 驱动的多引擎适配层**：

```text
Nekod Desktop (L1)
  Game Library · Mods · Trainers · Dependencies · Profiles · Permissions
       │
       ▼
Nekod Core (L2)
  Game & Engine Detection · Mod Manager · Dependency Manager · Compatibility DB · Security
       │  (Unified Runtime Interface: sdk/runtime-api)
       ▼
CompositeRuntimeRouter (L3)
  ├── Declarative / CE Script Runtime    (declarative_patch_v1 + .CT 转译/受限子集)
  ├── Dynamic Instrumentation Runtime    (Frida Gum 可选/隔离后端)
  ├── Unity Runtime Bridge               (BepInEx / MelonLoader — 区分 Mono 与 IL2CPP)
  ├── Unreal Engine Runtime Bridge       (UE4SS — UObject / Lua / C++ 反射桥接)
  └── Game-Specific Runtime Bridge       (REFramework / ScriptHook / RetroArch UDP 等)
```

**关键原则：统一的是管理契约（生命周期、权限、诊断、兼容性），而不是强行把五种底层技术揉成一种实现。**

| 统一管理操作 | 声明式 / CE Runtime | Frida Runtime | Unity Bridge | UE Bridge (`UE4SS`) | 专用生态 Bridge |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`attach()`** | 进程校验 & 附加 | 启动 Frida 会话 | 检测 Mono/IL2CPP 与 Loader | 检测 UE 版本 & UE4SS 兼容性 | 校验框架 / 启动回环 IPC |
| **`activate()`** | 写入内存 Patch / 冻结 | 挂载 JS Hook | 通过受控桥加载字段/方法修改 | 调用 `CheatManager` / 修改 `UObject` | 托管插件部署 / 发送协议命令 |
| **`deactivate()`** | `PatchLedger` LIFO 字节回滚 | 安全 `detach` 移除 Hook | 还原修改属性 & 卸载受控操作 | 还原 `UObject` / 卸载脚本 | `DeploymentLedger` 洁净还原 / 复位 |
| **`diagnostics()`** | 内存补丁数、冲突与冻结状态 | Hook 状态 & JS 日志 | Bridge IPC 状态与 Loader 日志 | UE4SS 日志 & 反射绑定状态 | 框架/协议连接与回滚报告 |
| **`capabilities()`** | `process.*` | Hook / 进程注入高风险权限 | 目录写入 + IPC + 反射作用域 | 目录写入 + UE4SS 依赖 + IPC | 按具体适配器独立声明 |

> **生态边界原则：** BepInEx、UE4SS、REFramework 本身已是成熟生态。Nekod 负责**发现、版本识别、配置管理、依赖调度与受控调用**，而**不需要把所有上游 Loader 从零重写一遍**。

---

## 2. 五类 Runtime Adapter 技术路线与当前进度

### 2.1 CE Script Runtime（优先考虑）
* **目标场景：** 大量存量老游戏、无明显现代引擎特征的原生 C/C++ 游戏，兼容现有 `.CT` 资源而无需玩家额外安装 Cheat Engine。
* **技术选型与边界：**
  - `.CT` 不只是普通 XML，还包含 Auto Assembler（`aobscanmodule`、`alloc`、`registersymbol`、`[ENABLE]`/`[DISABLE]`）、Lua 块、符号与复杂地址表达式。
  - **阶段一（已实现）：** 在 [ct_importer.rs](file:///d:/codespace/Nekod/tools/trainer-validator/src/ct_importer.rs) 提供静态 `.CT -> declarative_patch_v1` 转译器（`nekod-validator import-ct`），将模块偏移、反转后的指针链以及纯 `aobscanmodule` + `db` 字节补丁编译为经过校验的标准包；对 `{$lua}`、`alloc`、`createthread` 等默认拒绝并输出明细。
  - **阶段二（后续规划）：** 引入 `iced-x86` / 受限 AA 解析与剥离 `io/os/ffi` 的受控 LuaJIT 沙箱，支持常用 `.CT` 脚本子集并逐步扩大兼容性。

### 2.2 Dynamic Instrumentation Runtime（优先考虑）
* **目标场景：** 逻辑复杂的函数拦截、带条件分支的动态Hook与跨平台（Windows + Linux/Proton）运行时分析。
* **技术选型与边界：** `frida-gum`。开发者使用 JavaScript 编写 Hook，无需为每个 Mod 编译独立原生 DLL。
* **工程约束：** 不能把 `frida-gum` 误解为“随便引个库即可”。其静态链体积、跨平台适配、目标线程安全、`Interceptor.revert` 热卸载生命周期与杀软误报隔离都需要专门工程；应作为 **可选/隔离 Runtime Adapter** 独立发布与按需启用，不强行污染基础发行包体积与信誉。

### 2.3 Unity Runtime Bridge（优先考虑）
* **目标场景：** 覆盖海量独立/中型 PC 游戏的 Unity 生态，按类/字段/方法名修改，减少游戏小版本补丁带来的失效频率。
* **技术选型与边界：** 结合 `BepInEx` / `MelonLoader`，且 **必须显式区分 Mono 与 IL2CPP 两套运行环境**，不可假设单套桥接源码通吃两种运行时，同时在兼容性矩阵中追踪游戏版本更新引起的类结构变更。

### 2.4 Unreal Engine Runtime Bridge（建议支持）
* **目标场景：** UE4 / UE5 中大型游戏，利用 `UObject`、`UFunction` 反射与引擎内置 `CheatManager` 实现高稳定性修改。
* **技术选型与边界：** `UE4SS`（Lua / C++ 反射系统）。由于 UE 不同构建版本存在结构差异，UE4SS 并非对所有 UE 游戏开箱通用，必须在兼容性数据库中显式标注适用引擎版本、构建特征与已知限制。

### 2.5 Game-Specific Runtime Bridge（生态扩展）
* **目标场景：** Capcom RE 引擎（`REFramework`）、R 星生态（`ScriptHook`）、主机模拟器外部接口（`RetroArch UDP` 等）。
* **设计原则：** 每种专有桥接保持独立适配器与独立兼容性声明，绝不强行抽象成僵化的单一 Hook 接口。
  - **零注入外部协议（已落地阶段 1 底座）：** [adapter_ipc.rs](file:///d:/codespace/Nekod/crates/trainer-runtime/src/adapter_ipc.rs) 已实现 `adapter_ipc_v1`（含仅限 `127.0.0.1`/`localhost` 回环的 `retroarch_udp_v1` 命令执行、白名单动词校验与 LIFO 回滚）。
  - **目录代理 DLL 类（已有磁盘账本底座）：** [deployment.rs](file:///d:/codespace/Nekod/crates/trainer-runtime/src/deployment.rs) 已实现 `DeploymentLedger`，负责原文件隔离快照备份、SHA-256 投放验证与无冲突卸载还原。

---

## 3. 避免打补丁：下一阶段必须补齐的三大核心协议模块

为了不长期在面向数值修改器的 `TrainerManifest` 上打补丁，平台下一步将从 L4 协议层重构并补齐三大核心模块（对应 [ADR-019](file:///d:/codespace/Nekod/TECHNICAL_DECISIONS.md#adr-019-multi-engine-mod-platform-evolution-modmanifest-runtime-adapters-dependency-manager-and-compatibility-database)）：

### A. `ModManifest` & `Mod Package`（通用 Mod 清单，兼容并涵盖 Trainer）
将单一 Trainer 清单升级为多形态 **`ModManifest`**：
* **`trainer`**：数值修改、AOB 补丁、冻结项
* **`script_mod`**：Lua / JavaScript / 受控脚本 Mod
* **`plugin_mod`**：Unity / UE / 专有 Loader 托管插件 Mod
* **`asset_mod`**：模型、纹理、音频等资源替换包（由 `DeploymentLedger` 托管安全还原）
* **`configuration_mod`**：游戏配置 / INI / 参数调优 Mod
* **`patch_mod`**：文件级或二进制校验补丁
* **`emulator_mod`**：模拟器状态与网络命令扩展

> **向后兼容承诺：** 现有 v1 `TrainerManifest`（`schemas/trainer-manifest.v1.schema.json`）将提供自动向上映射为 `mod_kind = "trainer"` 的兼容转换器，确保现有示例与验证链路平滑演进。

### B. `Runtime Dependency Manager`（运行时依赖管理器）
不同 Mod 依赖不同的前置底座（`bepinex-mono`、`bepinex-il2cpp`、`ue4ss`、`reframework`、`lua-runtime` 等）。依赖管理器负责：
1. 在 `ModManifest` 中以版本约束声明所需 Runtime / Loader 依赖；
2. 扫描游戏目录与本地缓存，识别已安装依赖及版本；
3. 按 **上游许可证、再分发约束与用户显式授权** 决定“自动托管部署”或“引导至官方发布源安装 + 本地校验 SHA-256”，绝不违规打包或静默覆盖不可再分发二进制。

### C. `Compatibility Database`（多维兼容矩阵）
不再只记录 `Mod -> Game ID` 的粗粒度关系，而是建立精确兼容模型：

```text
Game
 ├── Game Version / Build ID
 ├── Distribution Platform (Steam / Epic / GOG / Custom)
 ├── Runtime Adapter (Engine + Loader + Version + Runtime Compatibility)
 └── Mod
      ├── Mod Version
      ├── Required Dependencies
      ├── Required Capabilities (PermissionScope)
      └── Tested Combinations (Verified / Experimental / Broken + Evidence)
```

---

## 4. 双生态协同与 AI Mod Forge 工作流

### 4.1 双生态边界
* **Player Ecosystem（玩家侧，零 AI 强制依赖）：** 游戏识别 → 引擎/Loader 探测 → 查看兼容矩阵与依赖状态 → 权限透明确认 → 一键启用/更新/回滚。
* **Developer / AI Ecosystem（开发者与 AI 侧，可选 L6）：** 利用 `crates/nekod-mcp`、`skills/` 与 `prompts/` 分析游戏、选择合适 Runtime、自动制作并验证 Mod 包，最终经人类开发者审校与授权后发布至社区仓库。

### 4.2 AI Mod Forge 流水线与防幻觉门禁

```text
输入游戏名称 / 游戏目录 / 进程 / 修改目标
   ▼
[1. Game Identification]     匹配 GameManifest / 商店 Build ID
   ▼
[2. Game & Engine Analysis]  识别原生 x64 / Unity (Mono vs IL2CPP) / UE4-5 / RE Engine / 模拟器
   ▼
[3. Runtime Recommendation]  结合兼容性数据库检索现成 Mod，推荐最优 Runtime Adapter
   ▼
[4. Mod & Manifest Forge]    生成或转译 Mod 代码/脚本 + ModManifest + 所需权限声明
   ▼
[5. Sandbox Validation]      nekod-validator Schema/安全检查 + 反作弊/单机策略校验
   ▼
[6. Compatibility Testing]   真实/沙箱运行时附加、激活、冲突测试与回滚验证（严禁跳过）
   ▼
[7. Community Publishing]    开发者人工审校、签名并提交 GitHub Release + Registry 索引
```

**强制门禁：** AI 分析出的内存地址、签名或脚本在通过第 5 步和第 6 步的**运行时实测与回滚验证**前，禁止自标为 `verified`，必须维持 `experimental` 或 `untested` 状态，并由开发者完成最终确认。

---

## 5. 分阶段实施顺序（不堆叠复杂度）

为避免同时开工五个高复杂度执行引擎导致失控，实施严格遵循“先定核心协议与管理底座，再逐个接入 Runtime”：

| 阶段 | 重点任务 | 当前状态 |
| :--- | :--- | :--- |
| **基础基线 (P0–P5 + V1.1-S1)** | 六层架构、SQLite 核心、Win32/Linux 声明式后端、`wasm_sandbox_v1` 宿主、`DeploymentLedger`、`.CT` 静态转译、`adapter_ipc_v1` | **已完成并测试通过** |
| **M1 — 核心协议重构** | 重新定义 `ModManifest`（v2 Schema）、扩展 `RuntimeProvider` 适配器接口与 `Mod Package` 规范（兼容 v1） | **下一步重点** |
| **M2 — 依赖与兼容数据库** | `Runtime Dependency Manager`、游戏引擎/Loader 探测增强、`Compatibility Database` 数据模型与存储 | **已规划** |
| **M3 — 传统与通用引擎桥接** | 受限 CE `.CT` 脚本子集执行器、`Unity Runtime Bridge`（Mono / IL2CPP）、`UE4SS` 桥接管理 | **已规划** |
| **M4 — 动态插桩与专有生态** | 可选隔离 `Frida Gum` Runtime、`REFramework` / `ScriptHook` 等游戏专用适配器 | **已规划** |
| **M5 — AI Mod Forge 闭环** | 扩充 `nekod-mcp` 游戏分析、Runtime 推荐、自动化生成与测试验证工具链 + 桌面 UI 体验升级 | **已规划** |
