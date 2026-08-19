# 应用结构

> 相关：[模块系统](modules.md) · [UI 组件](ui.md) · [变量与状态](variables-and-state.md)

## 应用入口：`main`

`fn main()` 是 Xulo 应用的入口点，根据返回值类型决定身份：

### UI 应用（返回 `View`）

```xulo
fn main(): View {
  Screen {
    VStack {
      Text("Hello, World!")
    }
  }
}
```

### CLI 脚本（无返回值）

```xulo
fn main() {
  print("Hello from Xulo")
}
```

### 规则

- `fn main(): View` → 根组件，返回 UI（由外部运行时通过 `__xulo_mount` 钩子挂载/渲染）
- `fn main()` (void) → 逻辑入口，执行脚本
- 一个文件只能有一个 `main` 函数
- 有 `main` 的文件是可执行应用，无 `main` 的文件是库模块

### 运行行为

| 文件类型 | `xulo run` 行为 |
|---------|----------------|
| 有 `fn main(): View` | 生成 JS，通过 `__xulo_mount` 钩子交由外部 UI 运行时挂载/渲染 |
| 有 `fn main()` | 执行脚本，输出到终端 |
| 无 `main` | 视为库模块，仅导出供其他文件使用 |

## 响应式运行时

当代码用到 `@State` / `@Store` / `@Effect` / `@Environment` / 组件时，编译器会在输出内联一个最小响应式运行时：

- `__signal(v)` → `{ get, set }` 响应式信号
- `__effect(fn, deps)` → 副作用（挂载时运行，信号变化重跑）
- `__component(render)` → 组件渲染器
- `__env(name)` → 环境值查找

`fn main(): View` 生成的挂载钩子：

```js
const __xulo_main = main();
if (typeof __xulo_mount === "function") __xulo_mount(__xulo_main);
```

真正的渲染 / 更新由外部 UI 运行时（如 `@xulo/ui`）负责。
