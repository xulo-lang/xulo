# 贡献指南

工程约定以 [`AGENTS.md`](AGENTS.md) 为准；本文件是面向人类贡献者的摘要。

## 分支

不要在 `master` 上直接提交。先同步、再建分支：

```bash
git switch master
git pull --ff-only origin master
git switch -c feat/your-change
```

分支名用 ASCII kebab-case，并带类型前缀：

| 前缀 | 用途 | 示例 |
|---|---|---|
| `feat/` | 新功能 | `feat/stage1-std-time` |
| `fix/` | 缺陷修复 | `fix/stage1-match-guard` |
| `refactor/` | 不改变行为的重构 | `refactor/xuloc-modules` |
| `test/` | 只改测试/语料 | `test/stage1-examples` |
| `docs/` | 只改文档 | `docs/git-branch-convention` |
| `chore/` | 构建、CI、依赖 | `chore/ci-macos` |

一个分支只做一件事。推送前 rebase 到最新 `origin/master`，不要 force-push
共享分支，合并后删除远端分支。

## 提交信息

沿用 `<type>(<scope>): <summary>`，类型与分支前缀一致，例如：

```text
feat(xuloc): 补齐 std::fs
fix(compiler): AOT 支持嵌套元组与元组传参/返回
docs: 约定 Git 分支工作流
```

## 合并前检查

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

涉及 stage 1（`xuloc/`）时再跑：

```bash
cargo test -p xulo-cli --test stage1
XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build
```

约定细节、测试位置与禁止事项见 [`AGENTS.md`](AGENTS.md)。
