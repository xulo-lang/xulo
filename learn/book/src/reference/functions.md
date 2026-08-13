# 函数

> 相关：[数据与类型](types.md) · [表达式](expressions.md) · [控制流](control-flow.md)

## 函数定义（Rust + TS 混合风格）

```xulo
// 无返回值
fn log(message: string) {
  print(message)
}

// 有返回值（隐式返回，Rust 风格）
fn add(a: number, b: number): number {
  a + b
}

// 显式 return
fn subtract(a: number, b: number): number {
  return a - b
}
```

**规则：**

- 用 `fn` 关键字（Rust 风格）
- 参数类型用 `:`（TS/Swift 风格）
- 返回值用 `:`（TS/Swift 风格）
- 最后表达式无分号 = 隐式返回（Rust 风格）；**声明了返回类型时，尾部表达式须与返回类型匹配**（否则语义错误）
- 也支持 `return` 显式返回
- 无返回值时省略返回类型

## 参数

### 可选参数（`?`）

```xulo
fn greet(name: string?): string {
  if name != null {
    "Hello, " + name
  } else {
    "Hello, stranger"
  }
}

greet()      // ✅ name = null
greet("A")   // ✅
```

### 默认参数值（`=`）

```xulo
fn greet(name: string = "stranger"): string {
  "Hello, " + name
}
```

### 命名参数

```xulo
fn Button(label: string, variant: string = "primary"): Component

Button(variant: "outline", label: "Submit")  // ✅ 命名参数，可乱序
```

- 调用时可用命名参数 `greet(name: "X")`；一旦使用命名参数，所有实参都须命名，且可乱序
- 字符串字面量联合类型（`type Status = "active" | "inactive"`）在实参处接受对应字面量

## 泛型函数

```xulo
fn first<T>(list: list<T>): T {
  list[0]
}

// 调用处推断类型实参：first([1, 2, 3]) 把 T 绑定为 number
let n: number = first([1, 2, 3])
let s: string = first(["a"])      // ✅ T = string
let bad: string = first([1, 2])   // ❌ 推断 T = number，与 string 不兼容
```

> 当前实现支持**调用处推断**泛型实参；**显式**类型实参（`first<number>(...)`）暂不支持。

## 匿名函数 / 闭包（Function Values）

`fn` 也可在表达式位置出现，作为值传递（闭包捕获外层作用域，等价 JS 闭包）：

```xulo
fn apply(f: fn(number): number, x: number): number {
  f(x)
}

fn makeAdder(n: number): fn(number): number {
  fn(v: number): number { v + n }   // ✅ 捕获 n
}

fn main() {
  let double = fn(x: number): number {
    x * 2
  }

  let add5 = makeAdder(5)
  print(apply(double, 3))   // 6
  print(add5(10))           // 15

  // 异步闭包：fn(): async
  let work = fn(): async { 42 }
  let v = await work()      // 42
}
```

**规则：**

- 匿名函数类型为 `fn(参数类型): 返回类型`（`Type::FnSig`），可赋值给带该类型的参数/变量
- 通过捕获自动访问外层局部变量，可变捕获可直接修改外层 `let` 绑定
- 调用函数值时只用位置实参，数量必须精确匹配；具名实参不支持
- 函数值可从任意表达式中调用：`xs[0](10)`、`getFn()(x)`、`(f)(5)`
- `fn(...)` 出现在语句位置（如块末用于隐式返回）时按匿名函数表达式解析
- 异步闭包写作 `fn(): async [类型]`，调用返回 Promise，可用 `await`
