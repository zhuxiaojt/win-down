<!--
You are currently viewing the English version of the PR template. To switch to the Chinese version, please add "?template=cn.md" parameter to the URL for creating the PR.

您正在查看英文版的PR模板，要想切换至中文版，请在创建PR的URL中添加?template=cn.md参数
-->

<!--
Thanks for considering a contribution to win-down!

Before submitting a PR, please read the contributing guide carefully!

When submitting a PR, the title should include the change type, in the format:
<type>: <description>

About <type>:
fix bug fixes
feat new features
docs documentation changes
refactor code refactoring
test test changes
chore build, tooling changes, and miscellaneous tasks

For example:
fix: fix concurrency scheduling issues in download scenarios

How to use the checkboxes below:
- [ ] unchecked checkbox
- [x] checked checkbox
-->

## Checklist

<!-- Before submitting the PR, please confirm the following. -->

- [ ] I confirm that my contribution follows the contributing guide
- [ ] I have run `npm run build` and confirmed it passes
- [ ] I have run `npm run test` and confirmed it passes
- [ ] Corresponding test cases have been added for download-scenario changes (if any)
- [ ] I have not committed any build artifacts or temporary files to the repository
- [ ] I confirm my code contains no leftover debugging code
- [ ] I have checked the "Allow edits and access to secrets by maintainers" option (if applicable)

## Scope of Changes

<!-- Before submitting the PR, please check the applicable boxes below. Multiple selections are allowed. -->

- [ ] Rust core engine
- [ ] TypeScript binding layer
- [ ] Build scripts
- [ ] Test cases
- [ ] Chinese/English documentation

## Description

- [ ] This PR contains breaking changes

<!-- If this PR contains breaking changes (e.g., API modifications), check the box above. -->

<!-- Next, describe your changes in detail here, including why they are needed and their impact on users. -->

## Related Issues

<!--
If any, please link the related Issues here.

For example:
- Close #1234

If there are no related Issues, remove this section.
-->
