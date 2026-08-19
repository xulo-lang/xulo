# UI 组件

> 相关：[变量与状态](variables-and-state.md) · [应用结构](application.md) · [编译器实现要点](../appendix/implementation.md)

## 基础组件（来自 `@xulo/ui` 标准库）

```text
VStack, HStack, ZStack  // 布局容器
Center                   // 居中容器
Text                     // 文本
Button                   // 按钮
Input                    // 输入框
Checkbox                 // 复选框
Link                     // 链接
Divider                  // 分割线
Card, CardHeader, ...   // 卡片组件
Screen                   // 根容器
```

## 组件使用

```xulo
// 带子元素的块语法
VStack(spacing: 16) {
  Text("Hello")
  Button(onClick: submit) {
    Text("Submit")
  }
}

// 无子元素直接用括号
Text("Hello", color: "blue")

// 嵌套
Screen {
  Center {
    Card(width: 400, radius: "lg") {
      Text("Welcome")
    }
  }
}
```

组件编译为 props 对象 + `children` 数组的调用：

```js
VStack({ "spacing": 16, children: [ Text({ "0": "Hello", children: [] }) ] })
```

- 具名实参成为 props 的键；位置实参按 `"0"`、`"1"` 折叠进 props；`children` 恒为数组。

## 自定义组件

```xulo
// 组件 = 返回 View 的函数
fn MyCard(title: string, children: list<View>): View {
  Card(radius: "md", shadow: "sm") {
    Text(title, weight: "bold")
    children        // 转发调用方传入的子元素
  }
}

// 使用自定义组件
MyCard(title: "Hello") {
  Text("Content goes here")
}
```

自定义组件（本地定义、返回 `View` 的函数）按**位置实参**调用：具名实参按声明顺序重排，`children` 传给名为 `children` 的参数。外部 `@xulo/ui` 组件则走 props 对象约定（见上节）。

## 表达式子元素

组件块内除了组件、裸字符串、`if`/`for`/分组外，还接受**任意表达式**作为子元素：

```xulo
VStack {
  children                                  // 转发 list<View>
  user.name                                 // string 值
  renderRow(item)                           // 调用产出 View 的表达式
}
```

- 表达式的类型必须是 `string`、`View`（或其可选形式）、`Any`，或元素类型为 `View`/`string`/`Any` 的 `list`；其他类型（如 `number`、`list<number>`）报错：`component children must be strings, components, or lists of components`。
- `list` 子元素渲染为嵌套数组，由运行时的渲染器展平（与 `if`/`for` 编译产物的约定一致）。

## 条件渲染

```xulo
if isLoggedIn {
  Text("Welcome back!")
} else {
  Button(onClick: login) {
    Text("Sign in")
  }
}
```

## 循环渲染

```xulo
for item in items {
  Text(item)
}
```

## Props（组件参数）

```xulo
fn Button(
  label: string,                     // 必传
  variant: string = "primary",       // 默认值
  icon: string? = null,              // 可选，默认 null
  disabled: boolean? = false         // 可选，默认 false
): View
```

- 必传参数：调用时必须传入
- 可选参数（`?`）：可传可不传
- 默认值（`=`）：可传可不传

## 事件处理

```xulo
// 函数引用
fn handleClick() {
  count = count + 1
}
Button(onClick: handleClick) {
  Text("Click me")
}

// 内联函数
Button(onClick: fn() {
  count = count + 1
}) {
  Text("Click me")
}

// 带参数的事件处理
fn handleInput(value: string) {
  name = value
}
Input(onInput: fn(e) { handleInput(e.value) })
```

## 主题系统

```xulo
// 主题 token（来自 @xulo/theme）
Text("Hello", color: "$theme.text.primary")
Text("Note", color: "$theme.text.secondary")
Button(variant: "$theme.button.primary")

// 色值
Text("Red", color: "#ff0000")
Text("Named", color: "blue")
```

- `$theme.*` 是字符串字面量，交由主题系统在运行时解释。

## 路由（标准库）

```xulo
import { Router, Route, Link } from "@xulo/router"

fn main(): View {
  Router {
    Route(path: "/", component: Home)
    Route(path: "/about", component: About)
    Route(path: "/profile/:id", component: Profile)
    Route(path: "*", component: NotFound)
  }
}

// 声明式导航
Link(to: "/about") {
  Text("About")
}

// 编程式导航
@Environment let router: Router
router.push("/about")
```
