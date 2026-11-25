# autocraft 设计文档

autocraft 是一个 Rust/GPUI 原生桌面应用，用来逆向分析软件（程序、应用、文件、网站），给出有证据支撑的回答，并按分析结果复刻出自己的版本。分析引擎是 craft-anything 的库 crate，在进程内运行；Agent 只支持 **pi、OMP、DSH**，它们通过一条 MCP 通道调用分析工具。

## 命名

| 用途 | 名称 |
|---|---|
| 产品名、仓库名 | `autocraft` |
| crate 前缀 | `autocraft-` |
| 可执行文件 | `autocraft.exe`；MCP 子命令是 `autocraft.exe mcp-host` |
| 环境变量前缀 | `AUTOCRAFT_` |
| 投影到引擎配置里的名字前缀 | `autocraft-`（MCP 条目、路由名） |
| 数据库 | `autocraft.db`（默认在 `<exe>/data/`，指定 `--data-dir` 时在该目录） |
| 项目与日志目录 | 默认 `%USERPROFILE%\.autocraft`，可用 `--data-dir <目录>` 改 |

## 外部依赖

| 依赖 | 来源 | 固定方式 |
|---|---|---|
| craft-anything 分析库 | `https://github.com/storytold/craft-anything` | Git 依赖，`rev` 固定到完整提交号（根 `Cargo.toml`，以 `ca-engine` 等别名引入） |
| LumaKit 共享组件 | `https://github.com/ouraihub/LumaKit.git` | Git 依赖，所有 LumaKit crate 用同一个 `rev`；更新步骤见 `AGENTS.md` |
| GPUI Kit | crates.io | 精确版本 `=0.11.0`，开 `gpui-fast` 与 `tree-sitter-c` 特性 |

## 文档

- [概要设计](概要设计.md)：范围、crate 分层、进程模型、状态归属、核心流程、界面。
- [详细设计](详细设计/README.md)：契约、分析服务、分析通道与引擎、模型与设置、复刻编排、界面、错误日志与测试。
- [构建](BUILD.md)：构建、打包、隔离运行、真实进程冒烟测试、运行时要求。
- [AI 工程规范](AI工程规范.md)、[日志与异常规范](日志与异常规范.md)：编码规则。
