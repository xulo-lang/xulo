# 表达式

> 相关：[控制流](control-flow.md) · [函数](functions.md) · [词法结构](lexical.md)

## 运算符优先级（高 → 低）

```text
后缀            f(x)  x[i]  x.y  x.0  x?.y  x?.0  x?
一元            !  -  ~  await  move  copy
幂              **（右结合）
乘除            *  /  %
加减            +  -
移位            <<  >>
位与            &
位异或          ^
位或            |
区间            ..<  ..=
比较            <  >  <=  >=
类型判断        is  is not
相等            ==  !=
空合并          ??
逻辑与          and
逻辑或          or
三目            ?:
赋值            =
```

优先级从高到低，完整规则见 `spec/expressions/operators.md`。

## 逻辑与三目

```xulo
let ok = a > 1 and b < 2       // `and` / `or`（不含 `&&` `||`）
let n = a > 1 ? "big" : "small" // 三目
print(!flag)                    // 逻辑非
```

## 字符串拼接

```xulo
let who = "Xulo"
print("Hello, " + who + "!")        // `+` 拼接字符串
print("count=" + str(count))        // `str(x)` 将任意值转成字符串后拼接
```

规则：

- `int + int` → `int`，`float + float` → `float`；`number` 按 `int | float` 的联合规则处理
- `string + string`（含字符串字面量类型）→ `string`
- `List<T> + List<U>` → 拼接成 `List`
- **禁止混用类型**：`"hi" + 1`、`"hi" + true` 等会报错；需要拼接时用内建 `str(x)` 显式转换后再 `+`。

## 成员访问 / 下标 / 可选链 / 空合并

```xulo
user.name           // 成员访问
xs[0]               // 下标
user?.name          // 可选成员访问（对象为 null 时得到 null）
let name = user?.name ?? "anonymous"   // ?? 空合并
```

## 列表展开

```xulo
let head = [1, 2]
let tail = [3, 4]
let all = [...head, ...tail]   // 展开合并
let with_extra = [...all, 9]
```

`...` 只能出现在列表/映射字面量内，展开操作数必须是 `List<T>`（列表）或 `Map<K, V>`（映射）。

## 映射展开

```xulo
let base = { a: 1 }
let copy = { ...base, b: 2 }   // 展开合并
```

## `match` 表达式

`match` 是表达式（详见 [控制流 · match](control-flow.md)）：

```xulo
let label = match status {
  0 => "zero"
  1 => "one"
  _ => "other"
}
```

## 函数值调用

函数值可从任意表达式调用：

```xulo
xs[0](10)        // 调用列表中的函数
get_fn()(x)       // 调用返回的函数
(f)(5)           // 调用括号包裹的函数值
```
