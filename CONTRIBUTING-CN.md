# 贡献指南

[English](https://github.com/zhuxiaojt/win-down/blob/main/CONTRIBUTING.md) | **简体中文**

感谢你对 win-down 感兴趣。win-down 是一个 Windows 专用、多线程的 Node.js 下载库：核心引擎由 Rust + WinHTTP 实现，通过 N-API 暴露给 JS，TypeScript 层负责公共 API、事件转发与 Promise/生命周期封装。

在动手之前，请先花几分钟读完这份文档。它能帮你判断改动应该落在哪一层、需要做哪些验证，以及如何写一个维护者容易合并的 PR。

---

## 1. 参与方式

不必为了「先获得许可」而开 Issue。按你的把握程度选择入口即可：

| 情况 | 建议做法 |
| --- | --- |
| 错别字、注释、文档修正，明确的单点 bug | 直接提 PR，无需先开 Issue |
| 改动明确、范围可控的小功能 | 直接提 PR |
| 方案没有把握、涉及引擎调度/续传/降级逻辑、跨 Rust 与 JS 边界 | 建议先开 Issue 对齐思路，减少返工 |

开 Issue 是可选的沟通手段，不是提 PR 的前提。**维护者不会因为缺少 Issue 而拒绝 PR**；但如果改动方向与项目定位不符，仍可能在评审中提出调整。

特别地，新建 PR 时建议勾选 **“Allow edits and access to secrets by maintainers”**，以便维护者直接修正细节，避免因小问题反复退回。

### 报告问题时请附上

- win-down 版本、Node.js 版本、Windows 版本
- 出错的 URL 及其服务器行为（是否支持 `Range`、是否重定向、是否走代理）
- 错误 `code`、相关日志与可复现的最小用例

---

## 2. 先定位层次

改动前请明确回答一句：**这次改的是哪一侧**

```
src/                  # TypeScript 绑定层
native/src/           # Rust 核心引擎
scripts/
  build.js            # cargo 构建并复制原生模块
  version.js          # 同步项目版本到每个清单文件
docs/                 # 中文 / 英文文档
test/                 # 本地 HTTP 服务 + 集成测试
```

- 下载、分段、重试、续传、校验、并发调度等**核心逻辑一律放在 Rust**（`native/src/engine/*` 核心模块及 `native/src/*` 相关模块）。
- TS 层（[src/index.cts](src/index.cts) 为实现，[src/index.mts](src/index.mts) 仅做 ESM 再导出）保持薄封装：公共 API、参数透传、事件转发、Promise 与对象生命周期管理。
- 不要把下载逻辑写进 TS，也不要在 Rust 里堆砌只为迁就 JS 调用的胶水代码。若确有必要在边界上做转换，请在 PR 描述里说明理由。

---

## 3. PR 描述

每个 PR 的描述至少回答三个问题：

1. **做了什么**：改动的行为与原因，而不是逐行复述 diff。
2. **如何验证**：跑了哪些命令、补了哪些用例、手工验证了什么场景。
3. **是否有破坏性变更**：有则写明影响范围与迁移方式；没有则明确写「无」。

标题使用类型前缀：

- `fix:` 修复 bug
- `feat:` 新增功能
- `docs:` 文档变更
- `refactor:` 代码重构
- `test:` 测试变更
- `chore:` 构建、辅助工具变更和杂务

如有关联 Issue，请在描述中引用。

建议**一个 PR 只处理一件事**，若一个 PR 中包含多件事，其中**任意**一事不符合上述要求、或不在项目计划内，整个 PR 都会被直接退回；拆开提交可以避免这类连带风险。

### 关于对外契约

跨 N-API 的枚举值、任务状态、事件名、错误码，**同时是 Rust 与 JS 两份对外契约**。例如 `TaskState`、`DegradeReason`、错误 `code`、事件 `kind` 等。修改这类内容时：

- 以 `native/src/core.rs`、`native/src/lib.rs` 为契约源，同步更新 `src/index.cts`（`src/index.mts` 只做 ESM 再导出，一般无需改动）；错误码与事件名还分散在 `native/src/engine/transfer.rs`、`native/src/engine/worker.rs`、`native/src/engine/scheduler.rs`，改动时请全局检索 `ERR_*` 与事件名；
- 同步更新对应文档（`docs/cn/`、`docs/en/`），README 与 docs 均为成对文件，改动需中英双语同步；
- 在 PR 中明确标注为破坏性变更。

只改一侧会导致运行时字段缺失或类型对不上，属于必须避免的情况。

---

## 4. 提交前

环境要求：Windows、Node.js >= 18、Rust 工具链（MSVC 工具链或可用的链接器）。

```bash
npm install     # 安装 typescript 等开发依赖
npm run build   # tsc（CJS + ESM），再用 cargo build --release 编译并复制为 native/win_down.node
npm run test    # 本地 HTTP 服务上的集成测试
```

`npm run test` 直接加载 `cjs/index.cjs` 与 `native/win_down.node`，未执行 `npm run build` 时无法运行。

- **只跑通一半等于没有验证**：`build` 覆盖 TS 与原生两侧，`test` 覆盖真实下载行为，二者都必须通过。
- 涉及下载场景的改动（分段、续传、降级、校验、取消、镜像容灾等），请在 [test/](test/) 中补充对应用例，并复用/扩展 [test/server.js](test/server.js) 的本地服务，避免依赖外网。
- 测试是仓库的一部分：修 bug 时优先补一个能复现该 bug 的用例。
- 构建产物（`cjs/`、`esm/`、`native/win_down.node`、`native/target/`）与 `test/tmp/` 已被忽略，请勿提交。
- 仓库未配置 CI，本地构建与测试是合入前的唯一关口，请在 PR 中贴出验证结果。
- 遵循现有代码风格，不要把无关的格式化或重排混入改动；TypeScript 编译开启了 `strict`。

---

## 5. 平台与原生约束

- **仅支持 Windows**：依赖 WinHTTP 与 Win32 API，不计划提供跨平台实现。
- 原生模块为 `native/win_down.node`（加载器还会回退查找包根目录的 `win_down.node`），加载路径与打包形态是设计输入。改到加载逻辑（[src/index.cts](src/index.cts) 中的 `loadNative`）时，请考虑真实部署场景，而不只是本地开发目录。
- 新增系统调用时，请控制在 `windows-sys` 已启用的 feature 范围内，或说明为何需要扩展。
- 涉及资源策略（`balanced` / `favor-other-apps` / `max-performance`）的改动，请说明对 CPU、内存与磁盘写入的实际影响。

---

## 6. 沟通与维护者预期

- PR、Issue 使用中文或英文均可。
- 本项目由维护者业余时间维护，**不承诺响应速度**，等待数周属于正常情况。
- 可以礼貌地催审，但催审不保证加速；请避免重复 @ 或刷屏。
- 评审中提出的问题，能改则改；若你认为评审意见有误，直接说明理由即可，欢迎讨论。

---

## 7. 底线要求

- **严禁恶意代码**：后门、窃取数据、隐藏的远程请求等一律拒绝。
- **安全漏洞请勿在公开 Issue 中披露**：先私下联系维护者，确认修复与披露节奏后再同步进展。
- 避免不文明语言与人身攻击。
- 维护者可以删除不当内容（评论、提交信息等），但不会因此退回整个 PR。

---

## 8. 许可证

本项目采用 [MIT 许可证](LICENSE)。提交贡献即表示你同意以相同许可证发布你的改动。
