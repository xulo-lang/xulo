# 项目约定 (Project Conventions)

## 不得修改 xulo-codegen
JS 代码生成已废弃并移除：工具链只使用 `xulo-runtime` 的原生解释器。
`crates/xulo-codegen`（以及 JS `bundle`/`build` 后端）不再参与任何执行路径；以后的修改**不需要、也不允许**改动 `crates/xulo-codegen`；相关功能改动应落在 `xulo-compiler`（或更上游的 lexer/parser/semantic）中。

## 测试位置
回归测试一律放在各 crate 的 `tests/` 目录（集成测试），不得与源码混合（禁止在 `src/` 内写 `#[cfg(test)]` 测试模块）。

## 验证
- 全量测试：`cargo test --workspace`
- CI 严格 lint：`cargo clippy --workspace --all-targets -- -D warnings`