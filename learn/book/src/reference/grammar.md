# 形式语法（EBNF）

> 形式语法正本见仓库 `spec/grammar.md`。本节为与本书内容对齐的简化版，差异见文末清单。

```text
(* Xulo Language Grammar (simplified) *)

(* ============================================================
   0. Document Root
   ============================================================ *)

document            = { import_stmt | pub_stmt | type_def | struct_def | enum_def | fn_def | const_stmt | let_stmt } ;
module              = document ;

(* ============================================================
   1. Comments (handled by lexer)
   ============================================================ *)

(*
   Lexer skips:
   - Line comments: // ... \n
   - Block comments: /* ... */
*)

(* ============================================================
   2. Identifiers & Literals
   ============================================================ *)

identifier          = (letter | "_") { letter | digit | "_" } ;  (* 关键字与保留字不可用作标识符，见 §lexical *)
type_identifier     = uppercase_letter { letter | digit | "_" } ;

string_literal      = '"' { character - '"' } '"' ;
number_literal      = digit { digit } [ "." digit { digit } ] ;
boolean_literal     = "true" | "false" ;
null_literal        = "null" ;

letter              = "A" | "B" | ... | "Z" | "a" | "b" | ... | "z" ;
uppercase_letter    = "A" | "B" | ... | "Z" ;
digit               = "0" | "1" | ... | "9" ;

(* ============================================================
   3. Types
   ============================================================ *)

type_expr           = union_type ;

union_type          = intersection_type { "|" intersection_type } ;
intersection_type   = primary_type { "&" primary_type } ;
primary_type        = "string"
                    | "number"
                    | "bool"
                    | "null"
                    | "object"
                    | type_identifier [ type_args ]      (* generic named type, e.g. Result<number> *)
                    | string_literal                     (* string literal type *)
                    | type_expr "?"                      (* optional type *)
                    | "List" "<" type_expr ">"         (* generic list *)
                    | "(" type_expr ")"
                    | "{" [ field_list ] "}"            (* object type *)
                    | "fn" "(" [ param_type_list ] ")" [ ":" type_expr ] (* function type *)
                    ;

type_args           = "<" type_expr { "," type_expr } ">" ;

field_list          = field_def { "," field_def } [ "," ] ;
field_def           = [ "pub" ] identifier ":" type_expr ;

param_type_list     = type_expr { "," type_expr } ;

(* ============================================================
   4. Type Definitions (type alias, struct, enum)
   ============================================================ *)

type_def            = "type" identifier [ type_params ] "=" type_expr ;
struct_def          = "struct" identifier [ type_params ] "{" [ field_list ] "}" ;
enum_def            = "enum" identifier [ type_params ] "{" enum_body "}" ;
enum_body           = enum_member { "," enum_member } [ "," ] ;
 enum_member         = identifier [ "(" [ payload_param { "," payload_param } ] ")" ] ;
 payload_param       = [ identifier ":" ] type_expr ;

type_params         = "<" type_param { "," type_param } ">" ;
type_param          = identifier ;

(* ============================================================
   5. Variables
   ============================================================ *)

const_stmt          = "const" identifier [ ":" type_expr ] "=" expr ;
let_stmt            = "let" ( ( "mut" identifier [ ":" type_expr ] )
                            | identifier [ ":" type_expr ]
                            | "{" identifier_list "}"
                            | "(" identifier_list ")" ) "=" expr ;

identifier_list     = identifier { "," identifier } [ "," ] ;

(* ============================================================
   6. Functions
   ============================================================ *)

fn_def              = [ "async" ] "fn" identifier [ type_params ] "(" [ param_list ] ")"
                        [ ":" type_expr ] block ;
param_list          = param_def { "," param_def } [ "," ] ;
param_def           = identifier ":" type_expr [ "=" expr ] ;  (* default value *)

block               = "{" { stmt } "}" ;

(* ============================================================
   7. Statements
   ============================================================ *)

stmt                = const_stmt
                    | let_stmt
                    | if_stmt
                    | for_stmt
                    | while_stmt
                    | return_stmt
                    | try_stmt
                    | throw_stmt
                    | expr_stmt
                    | block
                    ;

expr_stmt           = expr [ ";" ] ;

(* ============================================================
   8. Control Flow
   ============================================================ *)

if_stmt             = "if" expr block [ "else" ( block | if_stmt ) ] ;

for_stmt            = "for" identifier "in" expr block ;

while_stmt          = "while" expr block ;

return_stmt         = "return" [ expr ] ;

try_stmt            = "try" block "catch" "(" identifier ")" block ;

throw_stmt          = "throw" expr ;

(* ============================================================
   9. Expressions
   ============================================================ *)

expr                = ternary_expr ;

ternary_expr        = assign_expr [ "?" expr ":" expr ] ;

assign_expr         = logical_or_expr [ "=" assign_expr ] ;

logical_or_expr     = logical_and_expr { "or" logical_and_expr } ;
logical_and_expr    = nullish_expr { "and" nullish_expr } ;

nullish_expr        = equality_expr { "??" equality_expr } ;   (* nullish coalescing *)

equality_expr       = relational_expr { ("==" | "!=") relational_expr } ;
relational_expr     = additive_expr { ("<" | ">" | "<=" | ">=") additive_expr } ;

additive_expr       = multiplicative_expr { ("+" | "-") multiplicative_expr } ;
multiplicative_expr = unary_expr { ("*" | "/") unary_expr } ;

unary_expr          = ("!" | "-" | "await") unary_expr
                    | postfix_expr ;

postfix_expr        = primary_expr { postfix_op } ;

postfix_op          = "(" [ arg_list ] ")"         (* call: named callee, method, enum, or fn value *)
                    | "." identifier               (* member access *)
                    | "?." identifier              (* optional member access *)
                    | "[" expr "]"                 (* index *)
                    ;

primary_expr        = string_literal
                    | number_literal
                    | boolean_literal
                    | null_literal
                    | match_expr
                    | fn_expr                          (* anonymous function / closure *)
                    | "(" expr ")"
                    | "{" [ field_init_list ] "}"  (* object literal *)
                    | "[" [ expr_list ] "]"        (* list literal *)
                    | "{" block "}"                (* block expression *)
                    | identifier
                    | "_"
                    ;

fn_expr             = [ "async" ] "fn" "(" [ param_list ] ")" [ ":" type_expr ] block ;

match_expr          = "match" expr "{" { match_arm } "}" ;
match_arm           = match_pattern "=>" expr ;
match_pattern       = "_"
                    | string_literal
                    | number_literal
                    | boolean_literal
                    | type_identifier "::" identifier [ "(" [ identifier { "," identifier } ] ")" ] ; (* enum payload *)

(* ============================================================
   10. Call Arguments & Field Initializers
   ============================================================ *)

arg_list            = arg { "," arg } [ "," ] ;
arg                 = [ identifier ":" ] expr ;     (* labeled or positional *)

field_init_list     = field_init { "," field_init } [ "," ] ;
field_init          = identifier ":" expr
                    | "..." expr                     (* spread *)
                    ;

(* ============================================================
   11. Lists & Expression Lists
   ============================================================ *)

expr_list           = element_expr { "," element_expr } [ "," ] ;
element_expr        = expr
                    | "..." expr                     (* spread a list *)
                    ;

(* ============================================================
   12. Import / Export (Module System)
   ============================================================ *)

import_stmt         = "import" [ "type" ] import_spec [ "from" string_literal ]  (* "import type" is erased at runtime *)
                    | "import" [ "type" ] string_literal
                    ;

import_spec         = "*" "as" identifier
                    | "{" import_name_list "}"
                    | identifier "as" identifier ;

import_name_list    = import_name { "," import_name } [ "," ] ;
import_name         = identifier [ "as" identifier ] ;

pub_stmt            = "pub" pub_spec ;

pub_spec            = ( "const" | "let" | "fn" | "type" | "struct" | "enum" | "trait" ) declaration
                    | "use" "{" pub_name_list "}" ;

pub_name_list       = identifier { "," identifier } [ "," ] ;

(* ============================================================
   13. Full Source File
   ============================================================ *)

source_file         = { ( import_stmt | pub_stmt | type_def | struct_def | enum_def | fn_def | const_stmt | let_stmt ) } ;
```

## 与规范源的差异

本简化版相对 `spec/grammar.md` 的改动：

1. 只保留本书用到的核心子集，省略了模块树（`mod` / `use`）、`trait` / `impl`、属性、FFI、宏与并发等产生式。
2. `import_stmt` 保留书内使用的 `import ... from "..."` 写法，未列出规范源中的 `use` 路径形式。
3. `document` 与 `source_file` 合并列出顶层声明，不区分根模块与子模块。

## 实现与语法的已知差异

| 语法 | 实现 | 说明 |
|------|------|------|
| 相等 / 比较优先级 | 合并为一层 | 实现把 `== !=` 与 `< > <= >=` 合并在同一左结合层 |
| 显式泛型调用 `foo<T>()` | 不支持 | 仅支持调用处类型推断（见 [函数](functions.md)） |
