# autocraft

A Rust/GPUI desktop app that analyses a target (program, app, file, website) with evidence-backed answers and rebuilds it. The analysis engine is the craft-anything engine (Git dependency on `storytold/craft-anything`, pinned by `rev`) running in-process; the external agents pi, OMP and DSH drive it through one MCP channel and own their model loop, native history and built-in tools. Design lives in `docs/` (`README.md`, `概要设计.md`, `详细设计/`).

## Modules

A Cargo workspace of single-purpose crates. `core` holds the contracts; service crates implement its traits and depend on `core`; the app holds trait objects and is the only place concrete services are built (`crates/app/src/services.rs`).

Workspace layout:

| crate | role |
|---|---|
| `autocraft-core` | domain types, typed ids, service traits, error codes, session events; analysis/project/inbox contracts. No I/O, no UI. |
| `autocraft-analysis` | per-project analysis engine registry and the analysis channel server. Only this crate and `autocraft-rebuild` depend on craft-anything. |
| `autocraft-rebuild` | rebuild orchestration: research, build, check, converge. |
| `autocraft-agents` | pi / OMP JSONL and DSH ACP adapters, one `AgentRuntime` per backend; per-launch projection of skills, rules and MCP. |
| `autocraft-bridge` | authenticated named pipe and the `autocraft mcp-host` stdio forwarder. |
| `autocraft-routing`, `autocraft-settings` | provider routing, keys, gateway, MCP projection; one Turso database `autocraft.db`. |
| `autocraft-terminal`, `autocraft-observability`, `autocraft-ui` | PTYs; tracing and Beijing-time log files; visual tokens, fonts and stateless components. |
| `autocraft-testkit` | test fixtures shared by the workspace tests. |
| `autocraft` (app) | composition root and GPUI views. |

The composition root wires the per-project analysis service, authenticated MCP bridge, three external agents, rebuild coordinator and PTYs into the GPUI shell. The main area routes between Home, Project and Workbench; settings remain global.

Service crates never depend on each other. Engine adapters never call each other.

## Rust module and test organization

[`docs/AI工程规范.md`](docs/AI工程规范.md) is the baseline for every change. The rules below only extend it for this repository; where a rule here is stricter, the stricter rule applies. They apply to new code and to every production file substantially modified.
日志与异常的具体开发合同以 [`日志与异常规范.md`](docs/日志与异常规范.md) 为准；新功能必须明确最终错误记录者和跨线程 span 所有者，并读回真实输出验证级别、终态与脱敏，不能只确认存在日志宏。

### File responsibility and size

- Every `.rs` file MUST have one primary responsibility stated by its module comment or owning `SPEC.md`. Closely related private helpers MAY remain in the same file; unrelated UI, persistence, network, and domain concerns MUST NOT.
- A production file above 300 physical lines SHOULD be reviewed for responsibility before more code is added. A production file MUST NOT exceed 500 physical lines unless the change description records why extraction would make the boundary worse; generated, vendored, and fixture files are exempt.
- An existing file above 400 lines MUST NOT grow during an unrelated change. A feature change that crosses 400 lines MUST split the file in the same change, not defer the split.
- Line count is a review trigger, not the design. Split by independent state, dependency direction, lifecycle, or public contract—not by arbitrary chunks or one-function files.

### Directory modules and public surface

- A meaningful feature directory MUST use `mod.rs` as its coordinator and boundary. Sibling modules MUST be private by default; expose only the smallest `pub(super)` or `pub(crate)` surface required by the coordinator.
- The coordinator owns composition, routing between submodules, and feature-level state. It MUST NOT also own a second implementation of a submodule's persistence, probe, CRUD, or row-rendering logic.
- A submodule MUST NOT reach into another submodule's private fields. Pass an intent-specific method or narrow data type instead. Do not solve a boundary problem with `pub` fields, duplicate state, re-exports, or a second implementation.

`crates/app/src/panels/settings/providers/` is the concrete example: `mod.rs` coordinates `rail.rs` (provider list), `detail.rs` (provider detail), `keys.rs`, `models.rs` and `dialogs.rs`.

### Tests

Follows AI 工程规范 §8 and §10, which match the Rust community convention:

- **Unit tests** live in `src` next to the code they test, in a `#[cfg(test)] mod tests` block; when the block grows, move it to a sibling `<module>/tests.rs` or `<module>_tests.rs` registered with `#[cfg(test)] mod tests;`. Unit tests MAY use private items, but production code MUST NOT be made public only to be reachable from a test.
- **Integration tests** live in the crate's `tests/` directory and use only the public API. Shared helpers go in `tests/common/mod.rs`, never in a `tests/common.rs` without test functions.
- Production modules MUST NOT depend on test-only code, fixtures, or helpers.
- Tests MUST cover observable behavior, state transitions, errors and boundaries at the narrowest owning level. Do not add tests that pin layout text, forwarding copies, wiring, or incidental defaults.
- A test file MAY exceed the ordinary production-file target when its fixtures are inherently large, but it still MUST remain organized by contract and SHOULD be split before 500 lines.

`clippy.toml` allows `unwrap`, `expect` and printing inside `#[test]` functions only. Integration test crates also put helpers outside `#[test]` functions, so each file under `tests/` opens with `#![allow(clippy::unwrap_used, clippy::expect_used, reason = "…")]`. That exemption MUST NOT appear in `src`.

### Required pre-edit review

Before adding a non-trivial feature to a Rust module, the author MUST:

1. list the file's current responsibilities and physical line count;
2. identify the state owner, service boundary, and every affected caller;
3. decide whether the change belongs in the coordinator or a submodule;
4. place unit tests beside the code and integration tests under the crate's `tests/`;
5. run the affected check after the split, not only before it.

The implementation MUST finish with no stale duplicate path, compatibility copy, or unused extracted module.

### Match arms over enums

A `match` on an enum this workspace owns MUST name every variant, so adding a variant is a compile error at every decision point instead of a silent fallthrough. A bare `_ =>` stays only where it is justified:

- the scrutinee is a third-party or open type (`serde_json::Value`, `serde_yaml::Value`, `turso::Error`, `std::io::ErrorKind`), protocol strings or numbers;
- the arm binds and passes the value through unchanged (`other => …`, `error => error`), so every variant takes the same path by design.

Check with `cargo-msvc.bat clippy --workspace --lib --bins -- -W clippy::wildcard_enum_match_arm`; every remaining warning must fall under one of the two cases above. The lint is not enabled in the gate because it also flags those justified arms.

### State ownership

The host is the only writer of application metadata. Engine native history is not converted into application state or rewritten across engines. Client layout belongs to each window. A new default never changes an existing chat's backend.

Settings are global across engines; effective MCP configuration is project-scoped. Default model preferences and the default Agent apply only to new chats. Configuration changes dispose idle engines (or defer until idle), preserve their native binding and restart only on the next send. Engines use their own default config directories; per-chat support files and gateway tokens are cleaned up after disposal, before another launch can use the same support directory. DSH user routes are merged without rewriting user YAML; OMP merges only host-owned `autocraft-` entries in `mcp.json`.

Direct dependencies use exact latest non-yanked stable releases where compatible; pre-release versions and Git snapshots are not silently substituted. The one first-party exception is LumaKit, pinned by full commit `rev` to its GitHub repository and never to a local path. Rust is pinned to 1.99.1, GPUI Kit to the crates.io 0.11.0 release with GPUI Fast, and the dependency graph is committed in Cargo.lock. Upgrades require a Windows native-window smoke and exercised protocol behavior, not only dependency resolution.

## Verification

Use disposable project/config/data roots. Unit tests exercise filesystem, Git, request correlation and state transitions. Smoke tests launch the actual host/SDK processes; native verification opens a real GPUI window and exercises input and output. A successful build alone is not a UI or backend behavior proof.
