# 项目约定 (Project Conventions)

## 不得修改 xulo-codegen
JS 代码生成已废弃并移除：工具链只使用 `xulo-runtime` 的原生解释器。
`crates/xulo-codegen`（以及 JS `bundle`/`build` 后端）不再参与任何执行路径；以后的修改**不需要、也不允许**改动 `crates/xulo-codegen`；相关功能改动应落在 `xulo-compiler`（或更上游的 lexer/parser/semantic）中。

## 测试位置
回归测试一律放在各 crate 的 `tests/` 目录（集成测试），不得与源码混合（禁止在 `src/` 内写 `#[cfg(test)]` 测试模块）。

## 验证
- 全量测试：`cargo test --workspace`
- CI 严格 lint：`cargo clippy --workspace --all-targets -- -D warnings`

## Git 分支与提交

`master` 只接受通过分支合并的改动：**任何修改或新增功能都先开分支，不直接在
`master` 上提交**（包括文档与 CI 改动）。

- **创建分支前先同步**：`git switch master`、`git pull --ff-only origin master`，
  再从最新的 `master` 创建分支；推送前再 rebase 到最新 `origin/master`。
- 分支名用 ASCII kebab-case，并带类型前缀：

| 前缀 | 用途 | 示例 |
|---|---|---|
| `feat/` | 新功能 | `feat/stage1-std-time` |
| `fix/` | 缺陷修复 | `fix/stage1-match-guard` |
| `refactor/` | 不改变行为的重构 | `refactor/xuloc-modules` |
| `test/` | 只改测试/语料 | `test/stage1-examples` |
| `docs/` | 只改文档 | `docs/git-branch-convention` |
| `chore/` | 构建、CI、依赖 | `chore/ci-macos` |

- **不设 `merge/` 类型**：前缀描述改动的性质，合并本身不是改动。只有需要同时
  验证多个分支的组合时，才从最新 `master` 建临时 `integrate/<topic>`，测试通过
  后快进合并并删除；不要在 `integrate/` 上做功能改动。
- **先 rebase 再测，测的就是合并后的树**：`git fetch origin`、`git rebase
  origin/master`，然后在**该分支**上跑验证。`git switch master && git merge
  --ff-only <branch>` 得到的树与刚验证的分支完全相同，所以不需要单独的 merge
  分支，也不需要“先合并到某处再测”。
- `master` 只用 `--ff-only` 快进合并，不产生 merge commit；推送后由 CI 再兜底。
- 一个分支只做一件事；提交信息沿用 `<type>(<scope>): <summary>`
  （例如 `feat(xuloc): 补齐 std::fs`）。
- 推送前跑过本文件的“验证”命令；不要 force-push 共享分支。
- 合并后删除远端分支。AI/agent 开始改动前必须先建分支，并在回复里说明分支名。

面向人类贡献者的简短版本在 `CONTRIBUTING.md`，两者冲突时以本文件为准。