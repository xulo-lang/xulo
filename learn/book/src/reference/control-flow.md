# 控制流

> 相关：[表达式](expressions.md) · [函数](functions.md) · [异步](async.md)

## `if` 表达式（Rust/Swift 风格）

```xulo
let max = if a > b { a } else { b }

if condition {
  // ...
} else if other {
  // ...
} else {
  // ...
}
```

`if` 用作表达式时，两分支的尾部表达式类型须兼容（否则「incompatible types」错误）。

## `for` 循环（Swift 风格）

```xulo
for item in items {
  print(item)
}

for i in 0..<10 {     // 范围（左闭右开）
  print(i)
}

for i in 1..=10 {     // 范围（闭区间，含右端）
  print(i)
}
```

## `while` 循环

```xulo
let mut count = 0
while count < 10 {
  count = count + 1
}
```

## `match`（Rust 风格）

```xulo
match value {
  0 => "zero"
  1 => "one"
  _ => "other"
}
```

- 各分支的尾部表达式必须相互兼容（与 `if` 两分支的规则一致）：类型互不兼容时静态报错
- 分支间分隔可选逗号或换行，两者均可
- 支持枚举成员与 payload 绑定：

```xulo
let result = Result::Ok(42)
match result {
  Result::Ok(value) => print("Got: " + str(value))
  Result::Err(msg) => print("Error: " + msg)
}
```

## 错误处理：`Result` 与 `?`

语言没有 `try` / `catch` / `throw`。可恢复的失败是普通值：函数返回内建的 `Result<T, E>`，调用者用 `match` 检查，或用后缀 `?` 提前返回：

```xulo
fn half(n: int): Result<int, string> {
  if n % 2 == 0 { Result::Ok(n / 2) } else { Result::Err("odd input") }
}

fn doubled_half(n: int): int {
  match half(n) {
    Result::Ok(v) => v * 2
    Result::Err(reason) => { print(reason); 0 }
  }
}

fn quarter(n: int): Result<int, string> {
  let h = half(n)?      // Err 时整个函数返回该 Result::Err
  Result::Ok(h / 2)
}
```

- `Result::Ok(value)` / `Result::Err(error)` 是仅有的两个变体
- `?` 用于 `Result<T, E>` 或 `T?`：成功时取出值，失败时从外层函数提前返回
- 不可恢复的失败用 `panic(...)`；它没有处理器
- 详细规则见 `spec/error-handling.md`
