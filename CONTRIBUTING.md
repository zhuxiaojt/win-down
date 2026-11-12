# Contributing Guide

**English** | [简体中文](https://github.com/zhuxiaojt/win-down/blob/main/CONTRIBUTING-CN.md)

Thanks for your interest in contributing to win-down. win-down is a Windows-only, multi-threaded downloader for Node.js: the core engine is implemented in Rust + WinHTTP and exposed to JS via N-API, while the TypeScript layer handles the public API, event forwarding, and Promise/lifecycle wrappers.

Before you start, please take a few minutes to read this document. It will help you decide which layer your change belongs to, what verification is required, and how to write a PR that is easy for the maintainer to merge.

---

## 1. How to Get Involved

You don't need to open an Issue just to "get permission first". Choose your entry point based on how confident you are:

| Situation | Suggested approach |
| --- | --- |
| Typos, comment/doc fixes, clear single-point bugs | Open a PR directly, no Issue needed |
| Small, well-defined features | Open a PR directly |
| Uncertain approach, or changes involving engine scheduling/resume/degradation logic or the Rust↔JS boundary | Open an Issue first to align on the approach and reduce rework |

Opening an Issue is an optional communication tool, not a prerequisite for a PR. **The maintainer will not reject a PR for lacking an Issue**; however, if the direction doesn't fit the project's positioning, adjustments may be requested during review.

In particular, when opening a PR it is recommended to check **"Allow edits and access to secrets by maintainers"**, so the maintainer can fix small details directly instead of bouncing the PR back and forth.

### When Reporting Issues, Please Include

- win-down version, Node.js version, Windows version
- The failing URL and the server's behavior (whether `Range` is supported, whether it redirects, whether a proxy is involved)
- The error `code`, relevant logs, and a minimal reproducible case

---

## 2. Locate the Layer First

Before making changes, answer one question: **which side am I changing?**

```
src/                  # TypeScript binding layer
native/src/           # Rust core engine
scripts/build.js      # cargo build + copy of the native module
docs/                 # Chinese / English documentation
test/                 # local HTTP server + integration tests
```

- Core logic such as downloading, segmentation, retries, resume, verification, and concurrency scheduling **always belongs in Rust** (the `native/src/engine/*` core modules and related `native/src/*` modules).
- The TS layer ([src/index.cts](src/index.cts) is the implementation; [src/index.mts](src/index.mts) only re-exports it for ESM) stays a thin wrapper: public API, option pass-through, event forwarding, and Promise/object lifecycle management.
- Don't write download logic in TS, and don't pile glue code into Rust just to accommodate JS calls. If a boundary conversion is truly necessary, explain why in the PR description.

---

## 3. PR Description

Every PR description should answer at least three questions:

1. **What was done**: the behavior and rationale of the change, not a line-by-line recap of the diff.
2. **How it was verified**: which commands were run, which test cases were added, which scenarios were manually checked.
3. **Whether there are breaking changes**: if yes, state the impact and migration path; if not, say so explicitly.

Title uses a type prefix:

- `fix:` bug fixes
- `feat:` new features
- `docs:` documentation changes
- `refactor:` code refactoring
- `test:` test changes
- `chore:` build, tooling, and miscellaneous tasks

If there is a related Issue, reference it in the description.

It is recommended that **one PR handles exactly one purpose**. If a PR contains multiple purposes and **any** one of them fails to meet the requirements above or falls outside the project's plans, the entire PR will be rejected outright; submitting them separately avoids this collateral risk.

### About the External Contract

Enum values, task states, event names, and error codes that cross the N-API boundary are **the external contract on both the Rust and JS sides**. Examples: `TaskState`, `DegradeReason`, error `code`, event `kind`. When modifying these:

- Treat `native/src/core.rs` and `native/src/lib.rs` as the source of truth and update `src/index.cts` accordingly (`src/index.mts` only re-exports ESM and usually needs no change); error codes and event names are also scattered across `native/src/engine/transfer.rs`, `native/src/engine/worker.rs`, and `native/src/engine/scheduler.rs` — search globally for `ERR_*` and event names when changing them;
- Update the corresponding documentation (`docs/cn/`, `docs/en/`); README and docs are paired files, so changes must be synced in both Chinese and English;
- Clearly mark it as a breaking change in the PR.

Changing only one side will cause missing runtime fields or mismatched types — this must be avoided.

---

## 4. Before Submitting

Environment requirements: Windows, Node.js >= 18, and a Rust toolchain (MSVC toolchain or another working linker).

```bash
npm install     # installs dev dependencies such as typescript
npm run build   # tsc (CJS + ESM), then cargo build --release, copied to native/win_down.node
npm run test    # integration tests against a local HTTP server
```

`npm run test` loads `cjs/index.cjs` and `native/win_down.node` directly; it cannot run without `npm run build` first.

- **Passing only half is not verification**: `build` covers both TS and native sides, `test` covers real download behavior — both must pass.
- For changes involving download scenarios (segmentation, resume, degradation, verification, cancellation, mirror failover, etc.), please add corresponding test cases in [test/](test/) and reuse/extend the local server in [test/server.js](test/server.js) to avoid depending on the external network.
- Tests are part of the repository: when fixing a bug, prefer adding a test case that reproduces it.
- Build artifacts (`cjs/`, `esm/`, `native/win_down.node`, `native/target/`) and `test/tmp/` are gitignored; do not commit them.
- The repository has no CI, so local build and tests are the only gate before merging. Please paste the verification results in the PR.
- Follow the existing code style and don't mix unrelated reformatting or reordering into your change; the TypeScript build runs with `strict` enabled.

---

## 5. Platform and Native Constraints

- **Windows only**: depends on WinHTTP and Win32 APIs; cross-platform support is not planned.
- The native module is `native/win_down.node` (the loader also falls back to `win_down.node` in the package root). Load paths and packaging forms are design inputs. When touching the load logic (`loadNative` in [src/index.cts](src/index.cts)), consider real deployment scenarios, not just the local dev tree.
- When adding system calls, stay within the `windows-sys` features already enabled, or explain why an extension is needed.
- For changes involving resource policies (`balanced` / `favor-other-apps` / `max-performance`), describe the actual impact on CPU, memory, and disk writes.

---

## 6. Communication and Maintainer Expectations

- PRs and Issues may be written in Chinese or English.
- This project is maintained in the maintainer's spare time. **No response-time guarantee** — waiting a few weeks is normal.
- Politely pinging for review is fine, but it doesn't guarantee faster turnaround; avoid repeated @ mentions or flooding.
- If review comments can be addressed, address them; if you believe a comment is wrong, just explain your reasoning — discussion is welcome.

---

## 7. Baseline Requirements

- **Malicious code is strictly prohibited**: backdoors, data exfiltration, hidden remote requests — all rejected outright.
- **Do not disclose security vulnerabilities in public Issues**: contact the maintainer privately first, then share progress once the fix and disclosure schedule are agreed.
- Avoid uncivil language and personal attacks.
- The maintainer may remove inappropriate content (comments, commit messages, etc.) without rejecting the entire PR for it.

---

## 8. License

This project is licensed under the [MIT License](LICENSE). Submitting a contribution means you agree to release it under the same license.
