# 变量与绑定

> 相关：[数据与类型](types.md) · [函数](functions.md) · [应用结构](application.md)

Xulo 用 `let`、`let mut` 与 `const` 声明绑定。`let` 默认不可变，只有 `let mut` 引入的绑定（以及经它到达的字段、下标）可以赋值。

## `let` 与 `let mut`

```xulo
let count = 0                 // 不可变绑定；类型推断为 number
let name: string = "Alice"    // 带类型标注
let mut total = 0             // 可变绑定
let maybe: string? = null     // 可选类型
```

**规则：**

- 形式为 `let 名字 = 表达式` 或 `let 名字: 类型 = 表达式`；初始化表达式必须提供
- 类型标注写在名字之后，用 `:` 分隔（TS 风格）；可推断时可省略
- `let` 不可变；`let mut` 可变。`mut` 属于绑定形态，不属于类型
- 绑定从声明结束处生效，直到所在代码块结束

## `const`（编译期常量）

```xulo
const APP_NAME = "Xulo"
const MAX_COUNT: number = 100
```

- `const` 引入编译期常量，同样不可变；没有 `const mut`
- 初始化表达式必须是常量表达式：字面量、对其他 `const` 的引用，或它们的算术 / 比较运算
- 调用函数（包括内建函数）不属于常量表达式
- 可在模块顶层或代码块内声明

## 赋值

```xulo
let mut n = 0
n = n + 1

let mut xs = [1, 2, 3]
xs[0] = 9
```

- 赋值形式为 `位置 = 表达式`；位置必须由 `let mut` 绑定（或其字段 / 下标）支撑
- 赋值的结果是 `unit`，通常作为语句使用
- 给 `let` 或 `const` 赋值是语义错误
- 没有复合赋值与自增 / 自减运算符：写完整的 `n = n + 1`

## 作用域与遮蔽

```xulo
let x = 1

if true {
  let x = 2      // 内层遮蔽外层绑定
  print(x)       // 2
}

print(x)         // 1
```

- 同一个代码块内不能重复声明同名绑定；嵌套代码块中可以遮蔽
- 绑定不提升：在声明之前读取同一块中的名字是语义错误
- 闭包捕获的是捕获时刻的绑定；之后出现的同名声明不会改变已捕获的名字

## 解构绑定

`struct` 值可以用一条声明拆成多个不可变绑定：

```xulo
struct Point { x: number, y: number }

let p = Point(x: 1, y: 2)
let { x, y } = p
print(x + y)     // 3
```

- 解构只展开一层字段；字段名即绑定名，不支持重命名
- `let mut` 不会让解构出的各个绑定可变；需要修改时保留原值，或直接声明 `let mut p`
