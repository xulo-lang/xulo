# 模块系统

> 相关：[应用结构](application.md) · [完整示例](../appendix/example.md)

## 导入 / 导出（`pub`）

```xulo
// 导出（pub 声明）
pub fn add(a: number, b: number): number {
  a + b
}

pub const PI = 3.14

pub type User = {
  name: string
}

pub enum Status {
  Active
  Inactive
}

// 名称再导出
pub use { add, PI }

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
| `import "..."` | 副作用导入 |

**导出形式：** 两种，全部以 `pub` 开头——`pub fn/const/let/type/enum`（声明级）、`pub use { a, b }`（名称再导出）。

## `pub` 关键字（公开可见性）

`pub` 是声明级修饰符，把声明标记为公开（对其他模块可见）：

```xulo
pub fn add(a: number, b: number): number { a + b }
pub const PI = 3.14
pub enum Status { Active Inactive }
pub type User = { name: string }
```

- `pub fn/let/const/type/enum` 只影响跨模块可见性，模块内照常使用。
- `pub use { a, b }` 把本模块已声明的名称再导出（通常配合 `import` 做转发），语义与 Re-export 一致。
- `pub fn main`、`fn main` 都被识别为程序入口。

> 注意：`import type` 写在 `import` 之后（`import type { ... }`），而非花括号内（`import { type ... }` 暂不支持）。
> 历史：`export` 与 `default` 关键字均已移除并成为保留字（Xulo 没有默认导出）；跨模块导出统一用上面的 `pub` 形态，导入统一按名称。旧代码的 `export default fn main` 直接改写为 `pub fn main` 即可。

## 导入解析规则

- 相对路径或本地存在的模块名（如 `./math`、`math`）→ 解析为本地 `.xulo` 文件，由原生运行时加载。
- 其余说明符（如 `@xulo/ui`）→ 视为外部包，按 `xupm.toml` 的依赖映射解析；只有版本、没有本地源码的依赖暴露不透明面，其成员绑定为 `null` 占位符。不经过 Node.js / ESM。
- `import type` 只供类型检查；运行时仅当导入名同时是运行时值（如 `enum`）时保留其值绑定，以便 `Kind::Admin` 可用，否则完全擦除；`pub enum` 同时导出运行时值与类型。
- 本地模块由原生运行时按拓扑序初始化，入口文件执行其 `main()`。
- 循环导入（A → B → A）会被拒绝。

## 跨模块调用

- 导入的函数保留参数名，支持具名实参调用。
- 导入不存在的导出名报语义错误。
- `trait` 是可导出的类型成员（`pub trait`），用 `import type { Trait } from "./mod"` 引入；`impl` 块不跨模块移植，需在引用该特征的模块内本地声明。
