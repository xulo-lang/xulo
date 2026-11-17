# Executable conformance cases

The pinned `spec/` submodule records the normative examples but currently has no
`// runs:` annotations. These local cases keep the runner's executable path
(T3c) exercised; each block is a complete program and the annotation is the
trimmed stdout of `xulo run`.

```xulo
// runs: 42
fn main() {
    print(6 * 7)
}
```

```xulo
// runs: one two
fn main() {
    print("one" + " " + "two")
}
```

```xulo
// runs: 3
fn main() {
    let xs = [1, 2, 3]
    print(xs.len)
}
```

```xulo
// runs: true
fn main() {
    print((1, "a") == (1, "a"))
}
```
