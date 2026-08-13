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
// 组件 = 返回 Component 的函数
fn MyCard(title: string, children: list<Component>): Component {
  Card(radius: "md", shadow: "sm") {
    Text(title, weight: "bold")
    children
  }
}

// 使用自定义组件
MyCard(title: "Hello") {
  Text("Content goes here")
}
```

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
): Component
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

fn main(): Component {
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
