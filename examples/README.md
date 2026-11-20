# examples/

这个目录同时是三个东西：

1. **语言示例** —— 每个目录覆盖一类语法，读一遍就能看到该特性的写法；
2. **stage 0 语法门禁** —— `cargo test --workspace` 里的 `examples` 测试会
   发现每个程序、`xulo check`、运行它并与记录的输出比对；
3. **stage 1 自举语料** —— 同一批文件被 `xuloc` 的 lexer/parser 扫描，
   差距记录在 `crates/xulo-cli/tests/stage1_examples_baseline.tsv`。

## 目录

| 目录 | 覆盖的语法 |
|------|-----------|
| `basics/` | 入口程序、词法（进制/转义/模板/注释/Unicode）、绑定与解构、运算符优先级、模板字面量、`if`/`for`/`while`/`match` |
| `functions/` | 递归、闭包与高阶函数、箭头闭包、泛型函数、参数模式（默认/具名/`mut`/`shared`/`mut self`）、`defer`、`move`/`copy` |
| `types/` | `struct`/`enum`/`trait`、可选类型与可选链、类型别名（含联合/交叉/字面量类型）、元组、定宽数值类型、泛型 `impl`/`where`/const 泛型 |
| `patterns/` | 字面量、区间、or、类型、binder、guard 与 `let` 解构 |
| `collections/` | 列表/对象、`Map`、`Set`、`Array<T, N>`、字符串成员与 `StringBuilder` |
| `modules/` | 跨文件模块、内联 `mod`、`use` 分组/别名/glob、`use type`、`pub` 可见性 |
| `concurrency/` | `async`/`await`、`Task` 组合、`spawn` 四种目标 + service、`shared`/`lock` |
| `macros/` | 函数宏、派生宏、属性宏、`ast::quote!` |
| `testing/` | `#[test]` 声明与选项组、`xulo test` 报表 |
| `ffi/` | `extern "C"`、`Ptr`、`pub extern fn` |
| `errors/` | `Result` 与 `?`、`panic`、除零、静态类型错误 |
| `std/` | `std` 各模块的完整用法 |

## 入口判据

- 分类目录下直接放的 `.xulo` 是**单文件程序**；
- 含 `main.xulo` 的子目录是**一个多文件程序单元**，同目录其它 `.xulo` 是
  支撑模块（必须被 `main.xulo` 的 `mod` 声明引用）。

发现逻辑在 `crates/xulo-examples/src/lib.rs`，`xulo-cli` 与 `xupm` 的测试共用
它，所以「什么算一个程序」只有一处定义。

## 运行

```bash
xulo run examples/basics/hello.xulo
xulo run examples/modules/main.xulo
xulo test examples/testing/test_functions.xulo
```

## 期望输出（侧车文件）

每个程序旁边有一个 `<name>.expected`（stdout）；stderr 非空时再加
`<name>.stderr.expected`。平台相关的输出用 `<name>.<os>.expected` 覆盖，
`os` 取 `windows` / `linux` / `macos`。

重新生成（**先 review diff**）：

```bash
XULO_BLESS=1 cargo test -p xulo-cli --test examples
```

## 头部指令

指令写在入口文件开头的注释块里：

| 指令 | 作用 |
|------|------|
| `@no-aot all:` / `@no-aot windows:` | 不参与 AOT 差分：原生构建必须失败，或者两条路径的输出仍然不一致；一旦一致，差分测试会要求删掉标记 |
| `@skip-aot all:` | 不参与 AOT 差分，也不检查能否构建（例如测试载体） |
| `@no-run windows:` | 该平台只 `xulo check`，不 `xulo run` |
| `@error E0201` | `xulo check` 必须失败并报该错误码，跳过运行 |
| `@panic 文本` | `xulo run` 必须非零退出且 stderr 含该文本 |
| `@test-expect 6 passed, 0 failed, 1 skipped` | `xulo test` 的摘要行 |

## 覆盖表

`coverage.tsv` 把 `spec/grammar.md` 的 Production index（194 个非终结符）
逐条映射到例子：

```
<production>	<path>	<probe>	<note>
```

- 第 2 列是相对本目录的路径，`-` 表示豁免；
- 第 3 列是必须在文件里出现的字面文本（`-` 表示无法机械探测）；
- 豁免（第 2 列为 `-`）必须在第 4 列给出 `trivia:` / `terminal:` /
  `abstract:` / `error-only:` / `reserved:` 开头的理由。

校验在 `crates/xulo-cli/tests/examples.rs::coverage_table_accounts_for_grammar`：
与 grammar 的 index 双向差集、路径必须存在、probe 必须出现、不得重复。

**边界**：这张表能证明「每条 production 都有归属、每条归属都真实存在」，
不能证明语义覆盖 —— 那部分靠 review。已知的实现缺口用 `error-only:` 标注
（当前是 `StructPattern`），并记录在 `docs/tasks.md` 的 T8 里。

## stage 1 语料

```bash
XULO_STAGE1_MODE=scan XULO_STAGE1_DIR=examples xulo run xuloc/main.xulo
```

每行一个文件：`<path>\t<lexer 状态>\t<parser 状态>`。`XULO_STAGE1_SKIP`
接受分号分隔的路径列表，用于跳过会让解析器耗尽调用深度的输入。
