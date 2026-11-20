# 详细设计

各 crate 的接口和行为。类型和 trait 以 `crates/core` 为准；文档与代码不一致时，先修正文档，再改代码。总体结构见 [概要设计](../概要设计.md)。

## 文件

| 文件 | 内容 |
|---|---|
| [01-契约.md](01-契约.md) | `autocraft-core`：id、作用域、会话契约、分析/项目/inbox/复刻四个服务 trait、DTO、事件、错误码 |
| [02-分析服务.md](02-分析服务.md) | `autocraft-analysis`：引擎实例模型、Session 构造、项目注册、目标共享、刷新、净室/深度、执行确认、设置传播 |
| [03-分析通道与引擎.md](03-分析通道与引擎.md) | `autocraft-bridge` 与 `autocraft-agents`：管道协议、`mcp-host`、三个引擎的注册方式、构建方权限钩子、DSH 长工具、插话、技能与规则 |
| [04-模型与设置.md](04-模型与设置.md) | `autocraft-routing`、`autocraft-settings`：数据库表、项目级模型选择、设置页 |
| [05-复刻编排.md](05-复刻编排.md) | `autocraft-rebuild`：状态机、研究、构建方会话、审批、检查、补充说明、只是问问、feed 与活动日志、用量、`rebuild_*` 工具、排队 |
| [06-界面.md](06-界面.md) | `autocraft`（app）与 `autocraft-ui`：外壳、主区域路由、首页、项目视图、工作台、命令面板、卡片、组件、设置页、多语言 |
| [07-错误日志与测试.md](07-错误日志与测试.md) | 错误码、日志事件、测试分层、真实进程冒烟 |

## 通用约定

- **阻塞语义**：服务方法都是同步阻塞的，只在 `state::run` 的后台任务里调用，不在 GPUI 主线程上调用。
- **并发**：trait 都是 `Send + Sync`，以 `Arc<dyn …>` 共享，具体类型只在 `crates/app/src/services.rs` 里创建。实现内部自己负责互斥；craft-anything 的 `Engine` 用它自己的 session 锁。
- **craft-anything 类型不出边界**：craft-anything 的类型只出现在 `autocraft-analysis` 和 `autocraft-rebuild` 内部。core 的 DTO 用 autocraft 自己的结构体，工具参数和结果用 `serde_json::Value`。
- **持久化格式**：
  - 项目文件（`craft.json`、`feed.json`、`inbox/`、`investigation.cany/`、`usage.json`、`build-activity.log`）用 craft-anything 的格式，由它 engine 层的 API 读写，autocraft 不另写格式。
  - autocraft 自己的数据只放在 `autocraft.db`。
- **命名**：标识符、环境变量、投影进引擎的名字一律用 `autocraft` / `AUTOCRAFT_` 前缀。
