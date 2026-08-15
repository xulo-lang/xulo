# 模块系统

> 相关：[应用结构](application.md) · [完整示例](../appendix/example.md)

## 导入 / 导出（TypeScript 风格）

```xulo
// 导出
export fn add(a: number, b: number): number {
  a + b
}

export const PI = 3.14

export type User = {
  name: string
}

export enum Status {
  Active
  Inactive
}

// 默认导出
export default fn main() {
  print("Hello")
}

// 导入
import { add, PI } from "math"
import type { User } from "types"
import * as math from "math"
import "core"
```

**导入形式：**

| 形式 | 含义 |
|------|------|
| `import { a, b as c } from "..."` | 命名导入 |
| `import type { T } from "..."` | 仅类型导入（运行时擦除） |
| `import * as ns from "..."` | 命名空间导入 |
| `import default from "..."` | 默认导入 |
| `import "..."` | 副作用导入 |

**导出形式：** `export fn/const/let/type/enum`、`export default fn`、`export { a, b }`。

> 注意：`import type` 写在 `import` 之后（`import type { ... }`），而非花括号内（`import { type ... }` 暂不支持）。

## 导入解析规则（打包器）

- 相对路径或本地存在的模块名（如 `./math`、`math`）→ 解析为本地 `.xulo` 文件，参与打包。
- 其余说明符（如 `@xulo/ui`）→ 视为外部包，原样生成 ESM `import`。通过 bare specifier 解析到 `node_modules`（`xulo run` 会把临时 JS 写到源文件同目录，Node 会从那里向上查找；`examples/node_modules/@xulo/ui` 提供一个无头 demo shim）。
- `import type` 只供类型检查；运行时仅当导入名同时是运行时值（如 `enum`）时保留其值绑定，以便 `Kind::Admin` 可用，否则完全擦除；`export enum` 同时导出运行时值与类型。
- **无运行时模块加载器**：每个文件编译为一个返回导出对象的 IIFE，依赖按拓扑序内联进同一个 JS 文件；入口文件加载时执行其 `main()`。
- 循环导入（A → B → A）会被拒绝。

## 跨模块调用

- 导入的函数保留参数名，支持具名实参调用。
- 导入不存在的导出名报语义错误。
