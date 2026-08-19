(* Xulo Language Grammar v1.0 *)

(* ============================================================
   0. Document Root
   ============================================================ *)

document            = { import_stmt | pub_stmt | type_def | enum_def | fn_def | const_stmt | let_stmt } ;
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

identifier          = (letter | "_") { letter | digit | "_" } ;
type_identifier     = uppercase_letter { letter | digit | "_" } ;
component_name      = uppercase_letter { letter | digit | "_" } ;

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
                    | "boolean"
                    | "null"
                    | "object"
                    | type_identifier [ type_args ]      (* generic named type, e.g. Result<number> *)
                    | string_literal                     (* string literal type *)
                    | type_expr "?"                      (* optional type *)
                    | "list" "<" type_expr ">"          (* generic list *)
                    | "(" type_expr ")"
                    | "{" [ field_list ] "}"            (* object type *)
                    | "fn" "(" [ param_type_list ] ")" [ ":" type_expr ] (* function type *)
                    ;

type_args           = "<" type_expr { "," type_expr } ">" ;

field_list          = field_def { "," field_def } [ "," ] ;
field_def           = identifier ":" type_expr ;

param_type_list     = type_expr { "," type_expr } ;


(* ============================================================
   4. Type Definitions (type alias, enum)
   ============================================================ *)

type_def            = "type" identifier [ type_params ] "=" type_expr ;
enum_def            = "enum" identifier [ type_params ] "{" enum_body "}" ;
enum_body           = enum_member { "," enum_member } [ "," ] ;
enum_member         = identifier [ "(" [ identifier ":" ] type_expr ")" ] ;

type_params         = "<" type_param { "," type_param } ">" ;
type_param          = identifier ;


(* ============================================================
   5. Variables
   ============================================================ *)

const_stmt          = "const" identifier [ ":" type_expr ] "=" expr ;
let_stmt            = "let" identifier [ ":" type_expr ] [ "=" expr ] ;


(* ============================================================
   6. Functions
   ============================================================ *)

fn_def              = "fn" identifier [ type_params ] "(" [ param_list ] ")"
                        [ ":" ( "async" [ type_expr ] | type_expr ) ] block ;  (* ": async" makes an async fn *)
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
                    | component_stmt
                    | state_stmt
                    | store_stmt
                    | effect_stmt
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
   9. State / Store / Effect (UI-only contexts)
   ============================================================ *)

state_stmt          = "@State" ("let" | "const") identifier ":" type_expr [ "=" expr ] ;
store_stmt          = "@Store" [ "const" ] binding_pattern "=" expr ;
effect_stmt         = "@Effect" fn_expr [ "," "[" [ expr_list ] "]" ] ;

binding_pattern     = identifier
                    | "{" [ binding_field { "," binding_field } [ "," ] ] "}" ;
binding_field       = identifier [ ":" identifier ] ;


(* ============================================================
   10. Expressions
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

fn_expr             = "fn" "(" [ param_list ] ")" [ ":" ( "async" [ type_expr ] | type_expr ) ] block ;

match_expr          = "match" expr "{" { match_arm } "}" ;
match_arm           = match_pattern "=>" expr ;
match_pattern       = "_"
                    | string_literal
                    | number_literal
                    | boolean_literal
                    | type_identifier "::" identifier [ "(" identifier ")" ] ; (* enum [payload] *)


(* ============================================================
   11. Call Arguments & Field Initializers
   ============================================================ *)

arg_list            = arg { "," arg } [ "," ] ;
arg                 = [ identifier ":" ] expr ;     (* labeled or positional *)

field_init_list     = field_init { "," field_init } [ "," ] ;
field_init          = identifier ":" expr
                    | "..." expr                     (* spread *)
                    ;


(* ============================================================
   12. Lists & Expressions Lists
   ============================================================ *)

expr_list           = element_expr { "," element_expr } [ "," ] ;
element_expr        = expr
                    | "..." expr                     (* spread a list *)
                    ;


(* ============================================================
   13. UI Component Block (syntax sugar for children)
   ============================================================ *)

component_stmt      = component_call [ component_block ] ;

component_block     = "{" { ui_element } "}" ;

(* UI Element: only UI-specific constructs *)
ui_element          = component_stmt
                    | text_literal_expr
                    | if_stmt
                    | for_stmt
                    | "{" { ui_element } "}"
                    ;

(* NOTE: state_stmt / store_stmt / effect_stmt are intentionally NOT allowed inside
   a UI block. They may only appear at the top level of a function returning
   `View` (see the language reference, "变量与状态"). *)

text_literal_expr   = string_literal ;   (* naked string literal in UI block *)

component_call      = identifier [ type_args ] [ "(" [ arg_list ] ")" ] ;


(* ============================================================
   14. Import / Module Exports (`pub`)
   ============================================================ *)

import_stmt         = "import" [ "type" ] import_spec [ "from" string_literal ]  (* "import type" is erased at runtime *)
                    | "import" [ "type" ] string_literal
                    ;

(* NOTE: the form `import type * as identifier` (without `from`) is a draft
   artifact and is not supported by the current implementation. *)

import_spec         = "*" "as" identifier
                    | "{" import_name_list "}"
                    | identifier "as" identifier ;

import_name_list    = import_name { "," import_name } [ "," ] ;
import_name         = identifier [ "as" identifier ] ;

pub_stmt            = "pub" pub_spec ;

pub_spec            = ( "const" | "let" | "fn" | "type" | "enum" | "trait" ) declaration
                    | "use" "{" pub_name_list "}" ;

pub_name_list       = identifier { "," identifier } [ "," ] ;

(* ============================================================
   15. Full Source File
   ============================================================ *)

source_file         = { ( import_stmt | pub_stmt | type_def | enum_def | fn_def | const_stmt | let_stmt ) } ;