# 应用结构

> 相关：[模块系统](modules.md) · [变量与绑定](variables-and-state.md) · [函数](functions.md)

## 应用入口：`main`

`main` 是可执行程序的入口，必须无参数，并返回 `unit`。三种合法写法：

```xulo
fn main() {
  print("Hello from Xulo")
}
```

```xulo
fn main(): unit {
  print("Hello from Xulo")
}
```

```xulo
async fn main() {
  let data = await load_data()
  print(data)
}
```

**规则：**

- 返回类型可省略（等价于 `unit`），也可显式写 `unit`；其他返回类型都不合法
- `main` 不接收参数，声明在根模块的文件作用域
- 一个程序只有一个入口 `main`；有 `main` 的文件是可执行应用，没有 `main` 的文件是库模块
- `main` 可以标记为 `pub`，但不影响它作为入口的身份
- 程序正常结束退出码为 0；运行时失败以非零状态结束

### 运行行为

| 程序形态 | `xulo run` 行为 |
|---------|----------------|
| 有 `fn main()` 或 `fn main(): unit` | 执行程序，输出到终端 |
| 有 `async fn main()` | 异步执行入口，等待其任务完成 |
| 无 `main` | 视为库模块，仅导出供其他模块使用 |

## 应用与库

入口文件通过 `mod` 声明子模块，并用 `use` 取用它们的导出；被组合的文件不需要 `main`：

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

- 只有入口文件声明 `main`；库模块导出 `pub` 声明供入口或其他模块使用
- `use` 绑定的函数保留参数名，因此可以按名称传参
- 模块由原生运行时按依赖顺序初始化，最后执行入口 `main`

## UI 是库的关注点

语言层没有专门的 UI 入口或特殊返回类型。需要界面的程序导入一个库，由库基于普通的 `struct`、`enum` 与函数值构建界面；当前仓库不提供这样的库，未来由 `std/ui` 承担。
