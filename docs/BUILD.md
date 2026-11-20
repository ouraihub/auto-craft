# 构建

工具链由 `rust-toolchain.toml` 固定为 Rust 1.99.1（含 `rustfmt`、`clippy` 组件，rustup 自动安装），依赖精确锁定在 `Cargo.lock` 中。

## 便携 MSVC

Windows 上统一用便携 MSVC（MSVC 14.44 + Windows SDK 26100，x64）构建，即使已经装了 Visual Studio Build Tools 也一样。脚本在 `scripts/msvc/`，取自 [`@ouraihub/desktop-kit`](https://github.com/ouraihub/desktop-kit)（MIT，许可见 `scripts/msvc/LICENSE`）。

首次使用，在仓库根运行一次（下载约 455 MB 的微软构建组件，需要 `uv` 和 `curl`）：

```powershell
scripts\msvc\setup-msvc.bat
```

默认安装到 `D:\Applications\msvc-portable\msvc`，整台机器只需一份。装到别处时设置 `PORTABLE_MSVC_ROOT` 指向含 `setup_x64.bat` 的目录。

之后所有 cargo 命令都经 `cargo-msvc.bat` 执行，参数原样传给 cargo：

```powershell
scripts\msvc\cargo-msvc.bat build                      # 开发构建
scripts\msvc\cargo-msvc.bat fmt --all                  # 格式化
scripts\msvc\cargo-msvc.bat test --workspace           # 单元测试与集成测试
scripts\msvc\cargo-msvc.bat clippy --workspace --all-targets -- -D warnings
scripts\msvc\cargo-msvc.bat run -p autocraft -- --data-dir D:\tmp\autocraft-data
```

`cargo-msvc.bat` 先经 `msvc-env.bat` 注入便携 MSVC 的 `PATH`、`LIB`、`INCLUDE`，并检查 `cl.exe`、`link.exe` 确实来自便携目录，再调用 cargo。直接运行 `cargo` 会用到 Git for Windows `usr\bin` 里的 `link.exe`（GNU coreutils 的 link 命令，不是链接器），报 `extra operand … Try 'link --help'`。Windows SDK 的 `rc.exe` 也来自这套工具链，`turso` 等依赖的构建脚本需要它。在 GitHub Actions 上（`GITHUB_ACTIONS=true`）同一脚本改用 runner 自带的 MSVC。

GitHub Actions 的 `ci` 检查在每次 push 和 PR 上依次运行 `scripts\check-names.ps1`、`fmt --all -- --check`、clippy 和 `test --workspace`，命令与本机相同；推送前先在本机跑一遍。

## 隔离运行

验证时使用新的 `--data-dir`，避免读取或改写正式数据。引擎配置目录取自 `USERPROFILE`（见下文运行时要求），需要完全隔离时把它也指向临时目录。设置数据库版本不匹配时拒绝打开，不自动覆盖或删除。

编译通过、真实引擎回环冒烟和原生窗口交互是三种不同的证据，不能互相替代。

## 真实进程测试

需要真实进程或网络的测试默认标为 ignored，按需运行。它们都使用隔离目录和本机回环模型，不访问付费上游。

ConPTY 生命周期：

```powershell
scripts\msvc\cargo-msvc.bat test -p autocraft-terminal windows_pty_lifecycle_smoke -- --ignored --nocapture
```

引擎握手（`crates/agents/src/smoke.rs`，隔离配置、占位模型，无凭据、不发模型请求）。按实际安装位置设置：

```powershell
$env:AUTOCRAFT_SMOKE_PI = 'C:/path/node.exe|C:/path/pi-coding-agent/dist/bundle/cli.js'
$env:AUTOCRAFT_SMOKE_OMP = 'C:/path/omp.exe'          # 或 'C:/path/bun.exe|C:/path/omp/src/cli.ts'
$env:AUTOCRAFT_SMOKE_DSH = 'C:/path/node.exe|C:/path/dsh/lib/bin.js'   # Node ≥ 24.9
scripts\msvc\cargo-msvc.bat test -p autocraft-agents smoke:: -- --ignored --nocapture --test-threads=1
```

同样的变量也驱动 `crates/agents/tests/probe_*.rs` 与 `p01_pi_loopback.rs`，它们对真实引擎记录协议帧。`crates/agents/src/install/tests.rs` 中的两项会真实下载 Node、Bun 和引擎包。

端到端（`crates/app/tests/`）：将 `AUTOCRAFT_T26_SPEC` 指向版本 1 的 JSON 启动配置，格式见 `analysis_smoke.rs` 开头；设置 `AUTOCRAFT_T26_EVIDENCE` 时把观测结果写成 JSON 到该目录。`rebuild_smoke` 还需要 `PATH` 上有 `node`/`npm`。

```powershell
scripts\msvc\cargo-msvc.bat test -p autocraft --test analysis_smoke -- --ignored --nocapture
scripts\msvc\cargo-msvc.bat test -p autocraft --test rebuild_smoke -- --ignored --nocapture
scripts\msvc\cargo-msvc.bat test -p autocraft --test mode_consent_smoke -- --ignored --exact official_mode_and_project_consent_matrix --nocapture
```

`mode_consent_smoke` 对三个引擎分别核对净室 / 深度模式下新会话的工具列表，以及执行审批的允许一次 / 拒绝 / 项目持久允许 / 跨项目隔离。

网站、应用和游戏的真实中转复刻验收需要用户事先同意，不以回环模型测试冒充。

## 打包 EXE

从仓库根目录运行：

```powershell
scripts\msvc\package-exe.bat          # dist\autocraft-x64.zip
scripts\msvc\package-exe.bat 0.1.0    # dist\autocraft-0.1.0-x64.zip
```

脚本用便携 MSVC 构建 `--workspace --release`，把 `autocraft.exe` 复制到 `dist\autocraft\`，再生成 ZIP。ZIP 里只有 `autocraft\autocraft.exe`，不收集发布目录中的 `data/` 或其他运行产物，也不删除它们。版本号只用于 ZIP 文件名。

## 换行符

`.gitattributes` 规定：普通文本文件在仓库和工作区都用 LF；`.bat` / `.cmd` 用 CRLF，并且只能是纯 ASCII，否则 cmd.exe 会解析错。`.editorconfig` 让编辑器按同样的规则保存。CI 的 `line-endings` 检查会拒绝仓库里存成 CRLF 的文本文件和含非 ASCII 字符的批处理文件。

这些规则不依赖本机的 `core.autocrlf`。工作区里若还留着 CRLF 文件，在工作区干净时运行一次：

```powershell
git read-tree HEAD; git checkout-index -f -a
```

## 已知问题

- **debug 构建移动 target 目录后窗口打不开**（`Creating DirectX render pipelines: 0x80070003`）：GPUI 的 debug 构建会从编译时记录的绝对路径读取 shader。重新编译这两个 crate 即可：`cargo clean -p gpui-fast-windows -p gpui-pre-windows`。release 构建会把 shader 编译进二进制，不受影响。

## 运行时要求

发布包不带 Agent 引擎。打开“设置 → 编码搭档”：已在 `PATH` 上的 `pi` / `omp` / `dsh` 会被检测到并标“系统”；没装的点“安装”，应用下载固定版本的 Node v24.22.2 或 Bun 1.4.4（校验 SHA-256），再把 pi 1.4.0、OMP 18.14.2、DSH 0.2.1-rc.5 装到 `<数据目录>/coding`，标“内置”。“设为默认”决定新聊天用哪个引擎；新建项目或新建聊天时没装引擎、没设默认，界面会给出引导按钮直接打开这一页。引擎使用自己的默认配置目录（pi `~/.pi/agent`、OMP `~/.omp/agent`、DSH `~/.dsh`）。中转地址、模型和 API 密钥在“模型服务”中配置；每个引擎用哪个服务商在“编码搭档”里选，也可以选“{引擎} 官方”继续用引擎原有的模型配置。

数据目录默认是 `%USERPROFILE%\.autocraft`（日志在其下 `logs/`），可用 `--data-dir <目录>` 替换。模型、提供方、密钥、MCP、默认 Agent 和聊天绑定存在 `<exe>/data/autocraft.db`；给了 `--data-dir` 时数据库在该目录下。数据库表结构版本与程序不一致时拒绝打开：来自更新版本时提示升级，来自早期开发版本时提示删除 `autocraft.db` 后重建。新项目默认建在“文档\autocraft”，可在设置“项目位置”中修改。

API 密钥明文存放在数据库中，复制或显示时才取出；请保护数据库文件。所有项目共用 OMP 的默认配置目录，autocraft 只改写其中 `mcp.json` 里 `autocraft-` 开头的条目，因此 MCP 集合以最后启动的项目为准。

## 共享组件

模型选择器、上下文用量、模型服务 UI、设计令牌和日志封装来自 LumaKit（`lumakit-*` crate），以 Git 依赖固定到同一个 LumaKit 提交，见根 `Cargo.toml`；更新步骤见 `AGENTS.md`。
