# Nekod

**简体中文** | [English](README.en.md) | [日本語](README.ja.md)

> ✨ **本项目完全使用 Gemini Neon 模型创建 :p**

**Modding games, together.**  
*Open-source Community Mod Platform (`v1.0.0`) — Discover, create, manage, and share singleplayer game modifications.*

[![License: MIT](https://img.shields.io/badge/License-MIT-38B788.svg)](LICENSE)
[![Release](https://img.shields.io/badge/Release-v1.0.0-111514.svg)](.github/workflows/release.yml)

Nekod 是一个面向普通玩家与模组创作者的开源单机游戏修改与模组管理平台。它采用 **Deep Emerald Obsidian** 专业工作台界面，支持多语言（**简体中文 / English / 日本語**），将独立 Mod、Cheat Engine `.CT` 表格、Unity（BepInEx Mono / IL2CPP）与 Unreal Engine（RE-UE4SS）声明式模组统一到开放、可校验、可回滚的 `.nmod` 开放标准之下。

> **双生态设计 · 各自纯粹：**
> - **普通玩家（零 AI / 零代码）：** 打开 Nekod 桌面端即可自动识别本地游戏、从社区商店一键安装 `.nmod`、多 Mod 同游叠加、自定义热键与点击穿透 RGB HUD 悬浮窗。**无需任何 AI 账号、API Key、MCP 或命令行。**
> - **进阶创作者 &「MCP 许愿造 Mod」（可选）：** 接入官方可选组件 `nekod-mcp.exe` 后，不会写代码的普通人也可以直接用自然语言向 AI **“许愿”**——AI 会自动只读探测游戏引擎与对象结构，在沙箱校验通过后为你锻造、验签并打包可直接拖入 Nekod 的 `.nmod` 模组。

---

## 核心特性 / Highlights

- **专业工作台与双层暗黑界面（Deep Emerald Obsidian）：** `#0B1B16` 深森林绿侧栏 + `#0B0E0D` 近黑曜石工作区 + `1px` 低对比度描边与 `#38B788` 薄荷翡翠绿状态高亮；开箱直达工作台，支持**简体中文 / English / 日本語**即时切换。
- **开箱即用官方 Mod 商店：** 内置官方模组仓库桶 [`OmaruKoma/Nekod-Mods`](https://github.com/OmaruKoma/Nekod-Mods)（源头可读、`SHA-256` 校验、自动关联 `game-manifest.json`），支持订阅任意第三方 `registry.json` 源。
- **多 Mod 同游戏叠加与精细控制台：** 同一款游戏可同时叠加注入多个 `.nmod`（例如“角色/玩法 Mod”与“本地化汉化包”同时启用），支持按 LIFO 顺序安全卸载与回滚（`PatchLedger`）。
- **免手动注入体验：** 支持 **启动游戏并注入（Play & Inject）**、自动附着（Auto-Attach）、点击录制全局热键胶囊、系统托盘后台常驻，以及窗口右上角全鼠标穿透的 **RGB ArrayList HUD 悬浮窗**。
- **内置可审计伴生桥接与加载器管家（`DeploymentLedger`）：**
  - **Unity Mono / IL2CPP（`v0.5.5`）：** 零 UnityExplorer 依赖，支持自动部署 BepInEx 与 `NekodUnityBridge`，提供字段锁定、Harmony 方法挂钩、骨骼自转大陀螺、杀戮光环、UI 对象显隐与包内验签贴图材质替换。
  - **Unreal Engine 4/5（`v0.7.3`）：** 支持自动部署 `RE-UE4SS` 与 `NekodBridge`，单线程帧泵稳定调度，支持运行期 UMG 文本汉化、属性锁定、锁脖自瞄、自动开火、世界空间解耦第三人称视角大陀螺、原生杀戮光环等。
- **Cheat Engine `.CT` 静态安全转换：** 将纯静态偏移/指针链/`aobscanmodule + db` 表格转为带 SHA-256 校验的声明式模组，**fail-closed 安全拦截** `{$lua}`、`alloc()` 与远程脚本。

---

## 快速上手 / Quick Start (普通玩家)

### 1. 下载与安装（Windows x64）

前往项目的 **Releases** 页面下载对应发行产物（可配合 `SHA256SUMS.txt` 校验文件摘要）：

- **安装版（推荐）：** `nekod-windows-x64-setup.exe`（NSIS 安装包，支持当前用户静默安装）
- **便携绿色版：** `nekod-windows-x64-portable.zip`（包含桌面端 `nekod-desktop.exe`、校验器 `nekod-validator.exe` 与可选 MCP 服务 `nekod-mcp.exe`）

```powershell
# 可选：验证安装包或压缩包完整性
Get-FileHash .\nekod-windows-x64-setup.exe -Algorithm SHA256
```

### 2. 四步开启单机模组

1. **发现游戏：** 打开 Nekod 后，左侧边栏会自动扫描 Steam / Epic / GOG 已安装游戏；绿色版或独立单机游戏点击左上角 `+` 选择游戏目录或 `.exe` 即可加入。
2. **获取 `.nmod` 模组：**
   - 在顶部切换到 **「模组商店」**，点击 **「同步全部仓库」**，一键安装官方 `OmaruKoma/Nekod-Mods` 提供的模组；
   - 或直接把本地 `.nmod` / `.ct` 文件**拖入 Nekod 任意页面**松开即可校验导入。
3. **（按需）一键部署加载器与桥接：** 若某款 Unity 或 Unreal 游戏需要伴生桥接，点击工作台右上角提示即可通过 `DeploymentLedger` 一键安装 BepInEx / RE-UE4SS 与伴生桥接，卸载时无痕清理并还原原文件。
4. **授权并游玩：** 首次注入前根据 Default-Deny 安全提示确认模组申请的权限范围，随后点击 **「启动游戏并注入」**（或先开游戏再点 **「一键注入 Mod」**），使用面板开关或自定义全局热键即可畅玩。

---

## 用 MCP 向 AI 许愿创造模组（零代码 Mod 工坊）

> **普通人也能亲手做出专属 Mod！**  
> 如果社区商店里暂时没有你想要的冷门单机 Mod，或者你想给某款游戏定制“无限金钱 / 角色替换 / 伤害倍率 / 运行期汉化”，你不需要学逆向、C# 或 Lua——只需要把 Nekod 自带的 **`nekod-mcp.exe`** 接入任意支持 MCP（Model Context Protocol）的 AI 助手，然后用**日常聊天的话向 AI“许愿”**。

### 第一步：准备 `nekod-mcp.exe`

- **如果你下载的是便携版（`nekod-windows-x64-portable.zip`）：** 解压目录下已经自带 `nekod-mcp.exe`。
- **如果你是源码编译：** 执行 `cargo build --release -p nekod-mcp` 即可在 `target\release\nekod-mcp.exe` 获得。

### 第二步：把 `nekod-mcp` 接入你的 AI 助手

在常见 AI 客户端的 MCP 配置文件中加入以下 JSON（请把 `"command"` 替换成你电脑上 `nekod-mcp.exe` 的**真实绝对路径**，Windows 下注意双反斜杠 `\\`）：

#### 方案 A：Claude Desktop / Antigravity / Qoder / Windsurf / 通用 MCP 客户端

```json
{
  "mcpServers": {
    "nekod": {
      "command": "D:\\Tools\\Nekod\\nekod-mcp.exe",
      "args": [
        "--data-dir",
        "C:\\Users\\你的用户名\\AppData\\Roaming\\org.nekod.desktop"
      ]
    }
  }
}
```

#### 方案 B：Cursor（`.cursor/mcp.json`）

```json
{
  "mcpServers": {
    "nekod": {
      "command": "D:\\Tools\\Nekod\\nekod-mcp.exe"
    }
  }
}
```

> **提示：**
> - 如果填写了 `--data-dir` 指向 Nekod 桌面端的数据目录（或临时目录），MCP 工具就能直接读取你本地的游戏列表与测试工作区；省略 `--data-dir` 时默认使用系统临时目录下的隔离沙箱 `nekod-mcp-dev`。
> - 接入并重启 AI 助手后，能看到 **`nekod` 下的 24 个工具**（例如 `list_installed_games`、`inspect_unity_assemblies`、`probe_unity_bridge`、`probe_ue4ss_bridge`、`forge_nmod_package`、`pack_nmod_archive`）即表示配置成功！

### 第三步：准备工作（1 分钟）

1. 在 **Nekod 桌面端** 中添加或选中你要做 Mod 的**单机游戏**。
2. 如果是 Unity 或 Unreal Engine 游戏，在工作台先点一下 **「部署伴生桥接」**（若缺 BepInEx / RE-UE4SS 会提示一键自动安装），然后启动游戏并进入实际可游玩关卡。
3. 打开接入了 `nekod` MCP 的 AI 对话窗口，开始**许愿**！

### 第四步：直接复制这些「许愿话术模板」发给 AI

#### 愿望 1：Unity 游戏通用修改器许愿（无敌 / 资源 / 移速 / 倍率）

> “请使用 `nekod` 的 MCP 工具检查我已经添加的 Unity 单机游戏《游戏名》（目录 `D:\Games\MyUnityGame`）：  
> 1. 先用 `inspect_game_environment` 和 `inspect_unity_assemblies` 只读分析游戏程序集里的角色血量、金钱/弹药、移速相关类名和字段；  
> 2. 我已经打开了游戏并且部署了 Nekod 桥接，如有需要请用 `probe_unity_bridge` 核实真实字段名和实例路径；  
> 3. 请调用 `forge_nmod_package` 为我生成一个包含【无敌锁定血量 / 99999 金币按钮 / 2倍移速滑块】的 `.nmod` 模组包，配好 `F5`/`F6` 默认热键，并用 `verify_forged_mod_package` 和 `pack_nmod_archive` 打包成单文件 `.nmod` 输出路径告诉我！”

#### 愿望 2：Unreal Engine 4/5（UE4SS）游戏修改 / 汉化许愿

> “请用 `nekod` MCP 检查我的 Unreal 单机游戏《游戏名》：  
> 1. 游戏已经启动且加载了 UE4SS 桥接，请用 `probe_ue4ss_bridge` 只读探测玩家控制器、角色 Pawn 和武器/背包对象的真实 UProperty / UFunction；  
> 2. 帮我使用 `forge_nmod_package` 锻造一个 `ue4ss_bridge_v1` 的 `.nmod` 模组，实现【无限体力、无后坐力、一键满道具】，并通过 `verify_forged_mod_package` 校验后用 `pack_nmod_archive` 导出 `.nmod` 文件。”

#### 愿望 3：把网上找来的 Cheat Engine `.CT` 表格变成安全 `.nmod`

> “我有一个从社区下载的 `D:\Downloads\Game.CT` 表格，请用 `analyze_ct_file` 先分析里面哪些功能是安全的静态基址/指针链/AOB 补丁、哪些是高危 Lua 或远程脚本；然后帮我把其中安全的修改项锻造并打包成一个干净可读的 `.nmod` 模组包！”

### 为什么「MCP 许愿」对普通人也是安全的？

`crates/nekod-mcp` 在设计上采用 **读写严格隔离 + 显式授权门禁**：

| 工具分组 | 只读安全探测（无副作用） | 受控创建 / 打包（强制显式授权 + 校验） |
| :--- | :--- | :--- |
| **游戏与引擎发现** | `list_installed_games`、`get_game_info`、`inspect_game_environment`、`recommend_runtime_adapter`、`inspect_unity_assemblies` | — |
| **实时桥接反射探测** | `probe_unity_bridge`（严格白名单：`ping` / `list_types` / `list_members` / `find_instances` / `dump_hierarchy` / `get_field`）、`probe_ue4ss_bridge`、`inspect_authorized_game_process`、`inspect_trainer_state`、`get_execution_logs` | — |
| **模组锻造与 `.CT` 分析** | `analyze_ct_file`、`validate_trainer_manifest`、`inspect_trainer` | `forge_nmod_package`、`pack_nmod_archive`、`create_trainer_manifest`、`generate_trainer_template` |
| **兼容与仓库发布校验** | `check_game_version`、`validate_compatibility`、`inspect_runtime_diagnostics`、`validate_repository`、`inspect_registry_entry` | `verify_forged_mod_package`、`prepare_trainer_release` |

- **AI 永远无法越权：** AI 不能静默修改游戏进程内存写入任意代码，不能自动赋予高危权限，不能修改社区信任等级，也不能绕过任何反作弊限制。
- **最终确认权永远归玩家：** AI 生成的 `.nmod` 包拖入 Nekod 桌面端时，仍会经历完整的 **JSON Schema 严格校验 + SHA-256 哈希核验 + Default-Deny 权限清单弹窗**，确认安全透明后才会被执行。

---

## 架构概览 / Six-Layer Architecture

| Layer | Name | Responsibility | Primary Location |
| :--- | :--- | :--- | :--- |
| **L1** | Desktop Application | Player-facing Tauri 2 + React + TypeScript workbench & HUD overlay (zero AI/MCP) | [`apps/desktop/`](apps/desktop/README.md) |
| **L2** | Application Core | Modular Rust monolith (games, trainers, profiles, Default-Deny permissions, registry client) | [`crates/nekod-core/`](crates/nekod-core/Cargo.toml) |
| **L3** | Trainer Runtime | Win32/Linux runtime router, Unity & UE4SS bridges, `PatchLedger`, `DeploymentLedger`, WASM sandbox | [`crates/trainer-runtime/`](crates/trainer-runtime/Cargo.toml) |
| **L4** | SDK & Protocol | Versioned `GameManifest`, `TrainerManifest`, `ModManifest` v2 / `nmod_v1`, Runtime API, Trainer SDK | [`sdk/`](sdk/manifest/Cargo.toml), [`schemas/`](schemas/README.md) |
| **L5** | Community Registry | Source-first GitHub bucket protocol (`OmaruKoma/Nekod-Mods`) & CLI validator | [`community/`](community/README.md), [`tools/trainer-validator/`](tools/trainer-validator/Cargo.toml) |
| **L6** | AI Developer Tooling | Optional `nekod-mcp` stdio JSON-RPC 2.0 server, Agent Skills & contributor prompt library | [`crates/nekod-mcp/`](crates/nekod-mcp/Cargo.toml), [`skills/`](skills/README.md), [`prompts/`](prompts/README.md) |

---

## 安全模型与单机边界 / Security & Singleplayer Policy

1. **Default-Deny 权限体系：** 每个模组必须在清单中显式声明所需能力（如 `process.discover`、`process.write_memory`），授权绑定 `(trainer_id, version, manifest_sha256)` 三元组；模组文件一旦变更必须重新授权。
2. **仅限单机 / 严格反作弊红线：** Nekod **严禁并拒绝**一切反作弊绕过、隐身注入、内核逃逸或线上多人作弊功能。若检测到游戏目录或进程存在受保护的反作弊特征，运行时将立即 fail-closed 拒绝注入。
3. **可审计回滚：** 运行期内存补丁由 `PatchLedger` 记录原字节/状态并按 LIFO 顺序安全回滚；游戏目录加载器与伴生桥接由 `DeploymentLedger` 登记哈希并完整还原。

详见 [`SECURITY.md`](SECURITY.md) 与 [`docs/security-and-permission-model.md`](docs/security-and-permission-model.md)。

---

## 贡献与源码构建 / Contributing & Building from Source

### 贡献 `.nmod` 模组到官方商店

真实游戏 `.nmod` 包已独立托管于 [**`OmaruKoma/Nekod-Mods`**](https://github.com/OmaruKoma/Nekod-Mods)，主仓 [`examples/`](examples/) 仅保留平台验证用最小示例包（`sample-singleplayer-rpg`、`unity-standalone-starter`、`unity-explorer-mcp-starter`）。欢迎按 [`CONTRIBUTING.md`](CONTRIBUTING.md) 向模组仓库提交干净可读的源头 `.nmod` 包。

### 从源码构建与运行测试

**环境依赖：**
- [Rust](https://rustup.rs/) >= 1.85（含 `rustfmt` 与 `clippy`）
- [Node.js](https://nodejs.org/) >= 20
- **Windows：** Visual Studio Build Tools（C++ 桌面开发负载）+ WebView2 Runtime
- **Linux：** `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`

```bash
# 1. 构建桌面端前端（React + TypeScript）
npm --prefix apps/desktop install
npm --prefix apps/desktop run build

# 2. 格式检查、Clippy 零告警与全量工作区测试
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3. 校验最小平台示例包
cargo run -p nekod-validator -- package examples/sample-singleplayer-rpg
cargo run -p nekod-validator -- nmod examples/unity-standalone-starter
cargo run -p nekod-validator -- nmod examples/unity-explorer-mcp-starter

# 4. 启动桌面端开发模式，或单独编译 Release 安装包 / MCP / 校验器
cd apps/desktop && npm run tauri dev
cargo build --release -p nekod-desktop -p nekod-mcp -p nekod-validator
```

---

## 文档导航 / Documentation

- **架构与设计决策：** [`ARCHITECTURE.md`](ARCHITECTURE.md)、[`TECHNICAL_DECISIONS.md`](TECHNICAL_DECISIONS.md)
- **路线图与引擎生态：** [`ROADMAP.md`](ROADMAP.md)、[`docs/v1.1-roadmap.md`](docs/v1.1-roadmap.md)、[`docs/runtime-engine-ecosystem.md`](docs/runtime-engine-ecosystem.md)
- **协议规范与 Schemas：** [`schemas/`](schemas/README.md)、[`docs/specs/game-manifest.md`](docs/specs/game-manifest.md)、[`docs/specs/trainer-manifest.md`](docs/specs/trainer-manifest.md)、[`docs/specs/registry-manifest.md`](docs/specs/registry-manifest.md)、[`docs/specs/declarative-patch.md`](docs/specs/declarative-patch.md)
- **AI 协作基准（给 Coding Agent）：** [`AGENTS.md`](AGENTS.md)

---

## License

Licensed under the [MIT License](LICENSE) (see [ADR-009](TECHNICAL_DECISIONS.md#adr-009-repository-license-mit)).
