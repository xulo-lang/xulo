use xulo_cli::formatter::format;

fn fmt(source: &str) -> String {
    format(source).unwrap()
}

#[test]
fn expands_blocks_and_indents() {
    let out = fmt("fn main(){print(1)\nif true{print(2)}}");
    assert_eq!(
        out,
        "fn main() {\n  print(1)\n  if true {\n    print(2)\n  }\n}\n"
    );
}

#[test]
fn normalizes_spacing_around_operators() {
    let out = fmt("let a=1+2\nlet b=a*3-1\nlet c=a==b\nprint(c)");
    assert!(out.contains("let a = 1 + 2"));
    assert!(out.contains("let b = a * 3 - 1"));
    assert!(out.contains("let c = a == b"));
}

#[test]
fn keeps_import_spec_inline() {
    let out = fmt("import {createStore} from \"@xulo/store\"\nfn main(){print(1)}");
    assert!(out.contains("import { createStore } from \"@xulo/store\""));
}

#[test]
fn keeps_pub_use_list_inline() {
    let out = fmt("pub use {add,PI}\nfn main(){print(1)}");
    assert!(out.contains("pub use { add, PI }"));
}

#[test]
fn keeps_destructure_inline() {
    let out = fmt("fn main(){ @Store const {user,theme} = store() }");
    assert!(out.contains("@Store const { user, theme } = store()"));
}

#[test]
fn no_space_in_call_index_and_member() {
    let out = fmt("fn main(){let xs=[1,2]\nxs[0]=9\nprint(xs[0])\nprint(user.name)}");
    assert!(out.contains("xs[0] = 9"));
    assert!(out.contains("print(user.name)"));
    assert!(out.contains("print(xs[0])"));
}

#[test]
fn range_and_optional_type() {
    let out = fmt("fn main(){for i in 0..<10{print(i)}}\nfn f(x: User?): number { return 1 }");
    assert!(out.contains("for i in 0..<10 {"));
    assert!(out.contains("fn f(x: User?): number {"));
}

#[test]
fn ternary_spacing() {
    let out = fmt("fn main(){let ok=true\nprint(ok?1:2)}");
    assert!(out.contains("print(ok ? 1: 2)"));
}

#[test]
fn empty_block_stays_inline() {
    let out = fmt("fn main(): Component { }\nfn empty() { }");
    assert!(out.contains("fn main(): Component { }"));
    assert!(out.contains("fn empty() { }"));
}

#[test]
fn empty_block_does_not_shift_sibling_indent() {
    // The `{ }` pair must not change depth, so a sibling statement after an
    // empty block keeps its indentation level.
    let out = fmt("fn main() { if true { } print(1) }");
    assert!(out.contains("if true { }\n  print(1)"), "got:\n{out}");
}

#[test]
fn is_idempotent() {
    let src = "fn fib(n:number):number{if n<=1{return n}else{return fib(n-1)+fib(n-2)}}\n";
    let once = fmt(src);
    assert_eq!(fmt(&once), once);
}

#[test]
fn comments_are_dropped() {
    let out = fmt("fn main() {\n  // a comment\n  print(1)\n}");
    assert!(!out.contains("comment"));
    assert!(out.contains("print(1)"));
}

#[test]
fn unary_minus_keeps_space_after_assign_return() {
    let out = fmt("fn f(): number { return -5 }\nfn main(){let x=-5\nprint(x)}");
    assert!(out.contains("return -5"));
    assert!(out.contains("let x = -5"));
}

#[test]
fn unary_minus_in_call_and_object_keeps_space() {
    let out = fmt("fn main(){let y=[1,-2,-3]\nprint(f(1,-2))\nlet o={a:-1}}");
    assert!(out.contains("[1, -2, -3]"));
    assert!(out.contains("print(f(1, -2))"));
    assert!(out.contains("a: -1"));
}

#[test]
fn unary_minus_after_open_paren_has_no_space() {
    let out = fmt("fn main(){print(-5)\nlet x=(-1)}");
    assert!(out.contains("print(-5)"));
    assert!(out.contains("let x = (-1)"));
}

#[test]
fn unary_minus_is_idempotent() {
    let src = "fn main(){let y=[1,-2,-3]\nprint(f(1,-2))}\n";
    let once = fmt(src);
    assert_eq!(fmt(&once), once);
}

#[test]
fn generic_type_annotation_gets_operator_spacing() {
    // Known D12: the formatter has no type context, so `<`/`>` inside a generic
    // type annotation are spaced like comparison operators, yielding
    // `list < number >`. It still re-parses; the deviation is cosmetic. Fixing
    // it requires distinguishing type annotations from comparisons. Pinned so
    // the behavior is deliberate.
    let out = fmt("fn main(){let xs: list<number> = []\nprint(1)}");
    assert!(out.contains("let xs: list < number > = []"), "out: {out}");
}

#[test]
fn comparison_operator_spacing_is_untouched() {
    let out = fmt("fn main(){let a=1\nlet b=2\nprint(a<b)}");
    assert!(out.contains("print(a < b)"), "out: {out}");
}
