<!--
You are currently viewing the Chinese version of the PR template. To switch to the English version, please remove the "?template=cn.md" parameter from the URL for creating the PR.

您正在查看中文版的PR模板，要想切换至英文版，请删除创建PR的URL中的?template=cn.md参数
-->

<!--
感谢您考虑为 win-down 作出贡献！

提交 PR 前，请仔细阅读贡献指南！

提交 PR 时，标题请包含改动类型，格式应为：
<type>: <description>

关于<type>：
fix 修复 bug
feat 新增功能
docs 文档变更
refactor 代码重构
test 测试变更
chore 构建、辅助工具变更和杂务

例如：
fix: 修复下载场景下的并发调度问题

下文中的复选框使用方法：
- [ ] 未勾选的复选框
- [x] 已勾选的复选框
-->

## 检查清单

<!-- 提交 PR 前，请确认以下事项。 -->

- [ ] 我已确认我的贡献遵循贡献指南
- [ ] 我已运行 `npm run build` 并确认通过
- [ ] 我已运行 `npm run test` 并确认通过
- [ ] 涉及下载场景的改动（如有）已补充对应测试用例
- [ ] 我没有将任何构建产物和临时文件提交到仓库
- [ ] 我已确认我提交的代码中没有遗留调试代码
- [ ] 我已勾选 “Allow edits and access to secrets by maintainers” 选项（如有）

## 改动位置

<!-- 提交 PR 前，请根据实际勾选以下复选框，可多选。 -->

- [ ] Rust 核心引擎
- [ ] TypeScript 绑定层
- [ ] 构建脚本
- [ ] 测试用例
- [ ] 中英文文档

## 改动描述

- [ ] 此 PR 包含破坏性变更

<!-- 若此 PR 包含破坏性变更（如 API 修改），请勾选上面的复选框。 -->

<!-- 接下来，在这里详细描述你的改动，包括为什么需要这个改动，以及它对用户的影响。 -->

## 关联 Issue

<!--
如果有，请在这里关联相关的 Issue

例如：
- Close #1234

若不关联 Issue，请删除此段。
-->
