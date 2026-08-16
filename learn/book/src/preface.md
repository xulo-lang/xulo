# 前言

Xulo 是一门 UI First 编程语言，语法吸收 Rust、Swift、TypeScript 的设计精华。

编译管线：

```text
.xulo 文件 → 词法分析 (Token) → 语法分析 (AST) → 语义检查 → 代码生成 (JS) → node 运行
```

## 设计目标

- **UI 优先**：把 UI 组件、响应式状态、副作用作为一等语言特性（`Component`、`@State`、`@Store`、`@Effect`）。
- **风格融合**：变量绑定取 TypeScript，函数与枚举取 Rust/Swift，状态管理取 SwiftUI/Zustand。
- **小而完整**：用 Rust + [winnow](https://github.com/winnow-rs/winnow) 实现一个能真正编译运行的前端，生成现代 JavaScript。

## 本书与 `docs/` 的关系

本手册（`learn/book`）是**面向使用者的整理版手册**；仓库根目录的 `docs/xulo-syntax.md` 与 `docs/xulo-ebnf.md` 是**底层语言规范源**。

- 手册内容从规范源整理而来，并已按当前实现做了一致性修正（例如 `import type` 的写法、`@State` 使用位置限制等）。
- 形式语法（EBNF）的正本见 `docs/xulo-ebnf.md`；本手册的 [形式语法（EBNF）](reference/grammar.md) 一节提供与实现对齐的修订版，并附差异说明。

## 语法风格来源

| 特性 | 风格来源 |
|------|---------|
| 变量绑定 `const`/`let` | TypeScript |
| 函数 `fn` | Rust |
| 类型标注 `: Type` | TypeScript / Swift |
| 隐式返回 | Rust |
| 可选类型 `T?` | Swift |
| 泛型 `<T>` | TypeScript / Rust |
| 联合类型 `T \| U` / 交叉类型 `T & U` | TypeScript |
| 枚举 `enum`（含关联数据） | Swift / Rust |
| 模块 `import`/`export` | TypeScript |
| `if` 表达式、`match` | Rust / Swift |
| `@State` / `@Effect` | SwiftUI |
| `@Store` | 新设计（Zustand 风格） |
| `$` 绑定 | SwiftUI |
| `async`/`await` | TypeScript |
| 组件块语法 `{ ... }` | SwiftUI |
| 应用入口 `main` | Rust/Go |
| 对象字面量 `({ ... })` | 自定义（消除歧义） |

## 如何阅读本书

- 第一次接触：先读 [第一部分 · 指南](guide/getting-started.md)，快速跑通一个程序。
- 需要查语法：直接跳 [第二部分 · 参考](reference/lexical.md) 对应章节。
- 需要一张总表：查 [速查表](cheatsheet.md)。
- 需要理解术语：查 [术语表](glossary.md)。
