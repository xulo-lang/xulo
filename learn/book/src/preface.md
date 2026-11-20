# 前言

Xulo 是一门静态类型系统编程语言，语法吸收 Rust、Swift、TypeScript 的设计精华。语言本身不内建 UI 框架：界面构造是库的关注点（未来的 `std/ui`），由普通的 `struct`、`enum` 与 `fn` 值搭建。

编译管线：

```text
.xulo 文件 → 词法分析 (Token) → 语法分析 (AST) → 语义检查 → 原生运行时执行
```

## 设计目标

- **系统语言优先**：类型检查、模块边界、可变性默认值与枚举匹配的完整性由语言保证，读者跳过代码也能依赖这些结构性质。
- **风格融合**：变量绑定取 TypeScript，函数与枚举取 Rust/Swift，模块用显式的 `pub`。
- **小而完整**：用 Rust + [pest](https://pest.rs) 实现一个能真正编译运行的前端，由原生运行时执行。
- **UI 属于库**：语言不定义视图树或状态原语；需要界面的程序像使用其他库一样导入库，用普通类型和函数构造数据。

## 本书与 `spec/` 的关系

本手册（`learn/book`）是**面向使用者的整理版手册**；仓库根目录的 `spec/` 是**底层语言规范源**。

- 手册内容从规范源整理而来，并已按当前实现做了一致性修正（例如 `use type` 的写法、`let` 默认不可变而 `let mut` 才可赋值等）。
- 形式语法（EBNF）的正本见 `spec/grammar.md`；本手册的 [形式语法（EBNF）](reference/grammar.md) 一节提供与实现对齐的简化版，并附差异说明。

## 语法风格来源

| 特性 | 风格来源 |
|------|---------|
| 变量绑定 `const`/`let` | TypeScript |
| 可变绑定 `let mut` | Rust |
| 函数 `fn` | Rust |
| 类型标注 `: Type` | TypeScript / Swift |
| 隐式返回 | Rust |
| 可选类型 `T?` | Swift |
| 泛型 `<T>` | TypeScript / Rust |
| 联合类型 `T \| U` / 交叉类型 `T & U` | TypeScript |
| 枚举 `enum`（含关联数据） | Swift / Rust |
| 模块 `mod`/`use`/`pub` | Rust |
| `if` 表达式、`match` | Rust / Swift |
| `async`/`await` | TypeScript |
| 应用入口 `main` | Rust/Go |
| 映射字面量 `{ ... }` / `({ ... })` | 自定义（消除与块表达式的歧义） |

## 如何阅读本书

- 第一次接触：先读 [第一部分 · 指南](guide/getting-started.md)，快速跑通一个程序。
- 需要查语法：直接跳 [第二部分 · 参考](reference/lexical.md) 对应章节。
- 需要一张总表：查 [速查表](cheatsheet.md)。
- 需要理解术语：查 [术语表](glossary.md)。
