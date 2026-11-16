# Nekod 多引擎运行时生态扩展设计（Runtime Engine Ecosystem）

> **定位与目标：** 将目前散落在网盘与独立工具中的修改生态（Cheat Engine `.CT`、Unity 反射插件、UE4SS、专有框架、模拟器接口以及可选动态插桩）统一收敛至 Nekod 的 **L3/L4 协议与权限边界** 之下——对玩家呈现统一的开关、热键、信任徽章与安全回滚体验；对开发者提供模块化、可扩展的 `RuntimeProvider` 接入规范。

---

## 1. 核心设计原则

1. **上层协议统一，底层按引擎分流：**
   - L1 桌面端（`apps/desktop`）与 L2 核心（`nekod-core`）始终只面向 `sdk/runtime-api` 中的统一抽象（`RuntimeProvider`、`FeatureCommand`、`GrantedPermissions`、`RuntimeDiagnostics`）。
   - L3（`crates/trainer-runtime`）通过 `CompositeRuntimeRouter` 按 `TrainerManifest.runtime.kind` 路由至对应运行时后端，避免向玩家暴露引擎差异。
2. **默认拒绝与风险分级（不妥协 `SECURITY.md` 底线）：**
   - 任何新增引擎都必须显式声明 `PermissionScope`，并在反作弊检测触发或竞技联机政策禁止时立刻中止。
   - **高敏/高权限扩展不得拖累主客户端信誉与杀软误报**：重型插桩（如 `frida-gum`）或原生脚本执行器应作为按需下载、进程隔离的可选扩展组件，而不是硬编码进主安装包的强制依赖。
3. **内存与磁盘双维度可回滚：**
   - 内存级补丁由现有 `PatchLedger` 管理（LIFO + 冲突检测 + PID 复用校验）。
   - 磁盘部署代理 DLL（如 BepInEx / UE4SS / REFramework）必须引入对称的 **洁净部署账本（`DeploymentLedger`）**，确保停用或卸载时游戏目录零残留。

---

## 2. 现有底座与协议扩展方向

当前 `sdk/runtime-api/src/lib.rs` 与 `crates/trainer-runtime/src/router.rs` 已确立 `RuntimeProvider` 与多后端分发机制。面向多引擎生态，规划以下扩展方言：

| 候选引擎 / 协议 | 规划 `RuntimeKind` / 工具定位 | 安全风险等级 | 核心适用场景 |
| :--- | :--- | :--- | :--- |
| **声明式字节/AOB/指针补丁** | `declarative_patch_v1` *(已实现)* | **Tier A（低）** | 常规单机游戏数值锁定、轻量字节补丁 |
| **WASM 沙箱逻辑** | `wasm_sandbox_v1` *(已内置底座)* | **Tier A（低）** | 需条件分支、状态机计算的跨平台安全脚本 |
| **外部协议适配器** | `adapter_ipc_v1` *(V1.1 优先)* | **Tier A（低）** | RetroArch UDP / 网络控制接口等零注入模拟器/工具 |
| **`.CT` 静态转译器** | L5/L6 CLI 工具链 *(V1.1 优先)* | **Tier A（低）** | 将纯 AOB / 字节补丁 / 指针链 `.CT` 离线编译为 `declarative_patch_v1` |
| **Unity 反射桥接** | `unity_bridge_v1` *(V1.1 重点)* | **Tier B（中）** | Mono / IL2CPP Unity 独立与中型游戏，按类名/字段读写抗版本更新 |
| **UE4 / UE5 反射桥接** | `ue4ss_bridge_v1` *(V1.1/V1.2)* | **Tier B（中）** | 基于 UE4SS 调用 `UObject` 属性与 `CheatManager` / `UFunction` |
| **专有成熟框架托管** | `managed_framework_v1` *(V1.2)* | **Tier B–C（中/高）** | REFramework（RE 引擎）、ScriptHook 等现有生态插件的受控托管部署 |
| **CE `.CT` 受限执行器** | `ce_subset_v1` *(V1.2 评估)* | **Tier C（高）** | 无法静态转译的存量复杂 `.CT`（受限 Auto-Assembler + 裁剪版 Lua 沙箱） |
| **嵌入式 Frida 动态插桩** | `frida_gum_v1` *(可选隔离包)* | **Tier C（高）** | 跨平台（Windows + Linux/Proton）复杂虚函数/分支逻辑实时 Hook |

---

## 3. 五大方向细化接入方案

### 3.1 Cheat Engine `.CT`：静态转译优先 + 受限沙箱兜底
直接复刻完整 CE（尤其是无限制 `LuaJIT` FFI/IO 与全量 Auto Assembler）维护成本极高且易引入 RCE 风险，因此拆为两阶段：
1. **阶段一（`.CT -> declarative_patch_v1` 离线编译器）：**
   - 在 `tools/trainer-validator` 与 `crates/nekod-mcp` 中提供解析器，提取 `.CT` XML 中的内存地址、多级指针偏移、`aobscan` / `aobscanmodule` 特征码与简单 `[ENABLE]`/`[DISABLE]` 字节替换，生成标准化 `trainer.json` 与 `trainer-manifest.json`。
   - 产物直接享受现有 `PatchLedger` LIFO 回滚与低风险信任模型。
2. **阶段二（受限 `.CT` 运行时子集 `ce_subset_v1`）：**
   - 基于纯 Rust `iced-x86` 进行指令边界分析与跳板重定位，仅开放白名单 Auto-Assembler 指令（`aobscanmodule`、`alloc`、`label`、`define`、`registersymbol`、`db`）。
   - 若内嵌 Lua 脚本，必须运行在剥离 `io` / `os` / `ffi` / `package.loadlib` 的受控沙箱内，仅暴露绑定 `GrantedPermissions` 的内存读写宿主机函数；同时强制要求玩家显式确认 `process.allocate_memory`。

### 3.2 动态插桩：嵌入式 `frida-gum`（可选进程隔离后端）
* **优势：** 对 Windows + Linux/Proton 复杂分支拦截能力最强，开发者可用 JS 编写高级 Hook 逻辑而无需为每个游戏编译原生 DLL。
* **防误报与隔离策略：**
  * 不纳入 `nekod-desktop` 默认发行二进制，避免主程序触发杀软启发式拦截或体积膨胀。
  * 作为可选扩展后端运行，挂载/卸载阶段必须保障线程安全的 `Interceptor.revert` 与异常恢复；默认对 `Unverified` 来源禁用，仅在高级设置及明确风险确认后启用。

### 3.3 Unity 反射桥接（BepInEx / MelonLoader + 通用受控桥接）
* **设计思路：** 避免玩家手动下载覆盖游戏目录。由 Nekod 托管维护并签名通用的 **`Nekod.UnityBridge`**（分别适配 Mono 与 Il2CppInterop），随平台安全目录受控部署。
* **命令白名单约束：**
  * Manifest 中声明目标类型/字段/方法路径（例如 `PlayerStats.health`、`InventoryManager.AddGold`），Bridge 通过本地命名管道 / IPC 接收 Nekod 核心的 `FeatureCommand`。
  * 通用 Bridge **只做声明式反射读写与安全调用，不加载未经 SHA-256 校验的第三方任意 C# 程序集**。

### 3.4 虚幻引擎反射桥接（UE4SS 托管底座）
* **版本化依赖托管：** UE4 / UE5 不同版本间对象数组与名称池布局存在差异，Manifest 允许声明经平台索引校验的 `ue4ss` 基线版本号与游戏配置文件哈希。
* **统一控制：** Bridge 通过本地 IPC 将引擎内 `CheatManager`、`UObject` 属性修改与 `UFunction` 触发映射为标准的 `toggle` / `action` / `integer` / `float` 控件，玩家无需接触开发者控制台或 Lua 源码。

### 3.5 专有引擎与模拟器协议（REFramework / ScriptHook / RetroArch UDP）
此类接口划分为两个安全子域分别接入：
1. **外部 IPC / 网络协议类（如 RetroArch UDP Network Command）：**
   - 零进程注入、零内存写入，只需通过轻量 `adapter_ipc_v1` 向本地回环端口发送经过白名单校验的状态命令；天然支持跨平台与最低权限运行。
2. **目录代理 DLL 类（如 REFramework `dinput8.dll`、ScriptHook）：**
   - 作为可版本校验、可追踪的托管兼容包接入，生命周期完全受控于下文 **DeploymentLedger**。

---

## 4. 关键基础设施：游戏目录“洁净部署账本”（`DeploymentLedger`）

BepInEx、UE4SS、REFramework、ScriptHook 等框架必须在游戏安装目录部署代理 DLL 或插件目录。若缺乏统一清理机制，极易出现“卸载客户端后游戏无法启动”的网盘 Mod 通病。因此需要在 L2/L3 之间建立磁盘级 `DeploymentLedger`：

1. **部署前快照：** 记录游戏目录原有同名文件（若存在则计算 SHA-256 并移入隔离备份区）。
2. **强校验投放：** 所有托管运行时文件投放时记录 `(game_id, session_or_package_id, relative_path, deployed_sha256)` 至 SQLite。
3. **禁止路径逃逸：** 所有部署路径沿用 `validate_relative_path` 严格限制在目标游戏安装根目录内。
4. **一键卸载与故障自愈：** 玩家停用桥接或清理游戏配置时，按账本逆序删除受管文件并恢复原备份；若发现目标文件已被游戏更新覆盖，只清理受管记录而不盲目删除未知新文件。

---

## 5. 分级安全模型映射（Tier A / B / C）

| 安全层级 | 包含运行时 | 授权策略 | 隔离要求 |
| :--- | :--- | :--- | :--- |
| **Tier A — 声明式与零注入协议** | `declarative_patch_v1`、`wasm_sandbox_v1`、`adapter_ipc_v1` | 常规用户授权确认；支持社区验证源 | 主核心/WASM 沙箱内运行，默认启用 `PatchLedger` |
| **Tier B — 官方托管反射桥接** | `unity_bridge_v1`、`ue4ss_bridge_v1`、`managed_framework_v1` | 显式确认目录写入/桥接注入 + 单机与反作弊检查 | 通用 Bridge 由官方/社区校验 SHA-256；配合 `DeploymentLedger` 洁净还原 |
| **Tier C — 动态插桩与原生脚本** | `ce_subset_v1`、`frida_gum_v1`、`native_provider_v1` | `default-deny`；需高风险显式确认与来源信任约束 | 可选下载扩展包 + 独立辅助进程 / 沙箱裁剪 |

---

## 6. 分阶段实施路线（V1.1 → V1.2）

### 阶段 1：V1.1 底座与高频生态（低风险、最快见效）
1. 实现 **`.CT -> declarative_patch_v1` 静态转换工具**（优先服务贡献者与本地包导入）。
2. 实现 **`DeploymentLedger`（磁盘洁净部署账本）** 及回滚单元测试。
3. 接入首批低注入协议适配器 **`adapter_ipc_v1`（RetroArch UDP 示例）**。
4. 原型验证 **`unity_bridge_v1`**（Mono 优先，随后补齐 IL2CPP）并接入 `CompositeRuntimeRouter`。

### 阶段 2：V1.2 主流 3A 与长尾兼容
1. 接入 **`ue4ss_bridge_v1`**（版本化 UE4/UE5 依赖与 `UObject` 声明式控制）。
2. 接入受管专有生态包（REFramework / ScriptHook 托管描述与一键还原）。
3. 评估落地 **受限 `.CT` 运行时子集（`ce_subset_v1`）** 与 **可选隔离 `frida_gum_v1` 扩展包**。

### 验收原则
每接入一个新的 `RuntimeProvider`，均须满足：
- 不破坏现有 `apps/desktop`（L1）前端契约与无 AI 离线体验；
- 具备完整的故障/崩溃/冲突回滚回归测试；
- 在 `tools/trainer-validator` 与 `schemas/` 中提供对应 versioned JSON Schema 与示例包。
