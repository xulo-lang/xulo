# 术语表

| 术语 | 定义 | 章节 |
|------|------|------|
| **绑定（Binding）** | 用 `let` / `let mut` / `const` 为值命名 | [变量与绑定](reference/variables-and-state.md) |
| **可变绑定** | 用 `let mut` 声明、可以再次赋值的绑定 | [变量与绑定](reference/variables-and-state.md) |
| **常量（Constant）** | `const` 声明的编译期求值绑定 | [变量与绑定](reference/variables-and-state.md) |
| **闭包（Closure）** | 捕获外层作用域的匿名函数值 | [函数](reference/functions.md) |
| **枚举（Enum）** | 一组固定值，可带关联数据 | [数据与类型](reference/types.md) |
| **联合类型** | `T \| U`，值为 T 或 U 之一 | [数据与类型](reference/types.md) |
| **交叉类型** | `T & U`，同时满足 T 与 U | [数据与类型](reference/types.md) |
| **可选类型** | `T?`，等价 `T \| null` | [数据与类型](reference/types.md) |
| **泛型** | `<T>` 参数化的类型 / 函数 | [数据与类型](reference/types.md) |
| **隐式返回** | 函数体末尾无分号表达式作为返回值 | [函数](reference/functions.md) |
| **命名参数** | 按参数名传参（可乱序） | [函数](reference/functions.md) |
| **解构** | `let { a, b } = expr` 取字段绑定 | [变量与绑定](reference/variables-and-state.md) |
| **模块** | 由 `mod` 声明、用 `use` 绑定其导出的 `.xulo` 文件 | [模块系统](reference/modules.md) |
| **入口（Entry）** | 根模块中无参数、返回 `unit` 的 `main` | [应用结构](reference/application.md) |
| **形式语法** | EBNF 描述的语言文法 | [形式语法](reference/grammar.md) |
