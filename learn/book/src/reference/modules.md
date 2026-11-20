# 模块系统

> 相关：[应用结构](application.md) · [完整示例](../appendix/example.md)

Xulo 的包是一棵由 `mod` 声明的模块树。`mod` 声明模块，`use` 把路径上的名字绑定到当前文件，`pub` 决定名字是否进入模块接口。路径统一用 `::`，不再有字符串说明符，也没有 `import ... from`。

## 声明与文件

```xulo
// main.xulo
mod form;
mod internal;

use form::validate;

mod tests {
  use form::validate;
}

fn main() {
  print(validate("x"))
}
```

- `mod form;` 声明一个子模块，并认领文件 `form.xulo`（嵌套模块声明为 `form/fields.xulo` 等）
- `mod tests { ... }` 在当前位置内联声明模块，花括号内同样以 `mod` / `use` 头开始
- `mod` 与 `use` 是**头部形式**：写在文件其他顶层声明之前；写在函数体内是 `E0605`
- `pub mod form;` 把命名空间 `form` 导出，其他模块可写 `form::...`

## `use`：把名字绑定到当前文件

```xulo
use form::validate;                    // 绑定 validate
use lsp-server::router::Router;        // 包名中的连字符保持原样
use lsp-server::router::Router as R;   // 别名只改变本地名字
use net::{Client, Request};            // 分组
use form::{validate, fields::Field};   // 分组条目可以继续走子路径
use lsp-server::*;                     // 通配：绑定目标的所有导出
use type net::User;                    // 仅类型位置可用（运行时擦除）
```

| 形式 | 含义 |
|------|------|
| `use a::b;` | 绑定最后一段 `b` |
| `use a::b as c;` | 绑定为 `c` |
| `use a::{b, c as d};` | 分组绑定 |
| `use a::*;` | 绑定 `a` 的全部导出 |
| `use type a::T;` | 只在类型位置可见的类型绑定 |

- 路径每一段都必须存在，绑定的名字必须是目标的导出：未知名 `E0601`，私有名 `E0602`
- `use` 不会重新导出，也不会改变目标模块的接口
- 只有一段的路径（如 `use lsp-server as ls;`）必须写 `as`，绑定的是命名空间本身

## `pub` 与再导出

`pub` 是声明级修饰符：

```xulo
pub fn add(a: int, b: int): int { a + b }
pub struct User { pub name: string, age: int }
pub enum Status { Active, Inactive }
pub trait Shape { fn area(self): float }
pub type Score = int
pub const PI = 3.14
pub let app_name = "xulo"
pub mod form;
```

`pub use` 把路径上的名字绑定到当前文件**并**加入接口：

```xulo
pub use net::http::Client;
pub use net::http::Request as Req;
pub use net::{Response, Body};
pub use lsp-server::*;
pub use { add, PI };        // 再导出本文件已有的名字
```

- 路径形式读取目标导出，未知 / 私有分别报 `E0601` / `E0602`
- 本地形式 `pub use { a, b }` 中的名字必须已经属于本模块接口，否则报错

## 可见性与初始化

- 未标 `pub` 的声明只在声明它的模块内可见
- 结构体字段默认私有；跨模块读取或构造字段时需要 `pub`
- 模块按依赖顺序初始化；被 `use` 的名字若需要运行时值，会在使用前完成初始化
- 循环依赖在类型检查阶段被拒绝

## 入口与库

带 `main` 的根模块是可执行程序；没有 `main` 的文件是库模块。入口文件通过 `mod` 组合其他文件，通过 `use` 取用它们的导出：

```xulo
// math.xulo —— 库模块
pub fn add(a: int, b: int): int { a + b }
pub const PI = 3.14

// main.xulo —— 应用入口
mod math;
use math::{add, PI};

fn main() {
  print(add(1, PI))
}
```

- 入口只声明一次，位于根模块的文件作用域
- 只有根模块的 `main` 被当作入口；子模块中的同名函数只是普通函数
- 导入的函数保留参数名，因此可以按名称传参
- 详细规则见 `spec/modules/`（`source-files.md` 与 `import-export.md`）
