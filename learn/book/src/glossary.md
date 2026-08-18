# 术语表

| 术语 | 定义 | 章节 |
|------|------|------|
| **Component** | UI 组件的返回类型；返回 `Component` 的函数是组件 | [UI 组件](reference/ui.md) |
| **@State** | 组件内局部响应式状态，赋值触发重渲染 | [变量与状态](reference/variables-and-state.md) |
| **@Store** | 全局响应式状态的解构绑定（Zustand 风格） | [变量与状态](reference/variables-and-state.md) |
| **@Effect** | 组件副作用（挂载时运行，可带依赖数组） | [变量与状态](reference/variables-and-state.md) |
| **@Environment** | 从外部运行时注入的值 | [变量与状态](reference/variables-and-state.md) |
| **Props** | 组件参数 | [UI 组件](reference/ui.md) |
| **绑定（`$`）** | 双向绑定，读 / 写 `@State` / `@Store` 变量 | [变量与状态](reference/variables-and-state.md) |
| **闭包（Closure）** | 捕获外层作用域的匿名函数值 | [函数](reference/functions.md) |
| **枚举（Enum）** | 一组固定值，可带关联数据 | [数据与类型](reference/types.md) |
| **联合类型** | `T \| U`，值为 T 或 U 之一 | [数据与类型](reference/types.md) |
| **交叉类型** | `T & U`，同时满足 T 与 U | [数据与类型](reference/types.md) |
| **可选类型** | `T?`，等价 `T \| null` | [数据与类型](reference/types.md) |
| **泛型** | `<T>` 参数化的类型 / 函数 | [数据与类型](reference/types.md) |
| **隐式返回** | 函数体末尾无分号表达式作为返回值 | [函数](reference/functions.md) |
| **命名参数** | 按参数名传参（可乱序） | [函数](reference/functions.md) |
| **解构** | `{ a, b } = expr` 取对象字段绑定 | [变量与状态](reference/variables-and-state.md) |
| **模块** | 一个 `.xulo` 文件，可 `import`/导出（`pub`） | [模块系统](reference/modules.md) |
| **IIFE** | 立即执行函数表达式；每个模块编译为返回导出对象的 IIFE | [编译器实现要点](appendix/implementation.md) |
| **响应式（Reactive）** | 状态变化自动触发依赖更新 | [应用结构](reference/application.md) |
| **信号（Signal）** | 响应式单元：`{ get, set }` | [应用结构](reference/application.md) |
| **依赖数组** | `@Effect` 后的 `[deps]`，声明依赖 | [变量与状态](reference/variables-and-state.md) |
| **形式语法** | EBNF 描述的语言文法 | [形式语法](reference/grammar.md) |
