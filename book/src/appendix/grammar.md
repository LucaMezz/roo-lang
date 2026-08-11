# Grammar Summary

An informal EBNF-style summary of fig's syntax, for quick reference. This
is a summary of the constructs documented in the rest of the book, not a
formal, implementation-grade grammar — precedence, whitespace, and comment
handling are omitted; see the linked chapters for full semantics.

```ebnf
(* -------------------------------------------------------------- *)
(* A file *)

program     ::= statement*  (* a fig file runs top to bottom, like a script —
                                no entry-point function required *)

(* -------------------------------------------------------------- *)
(* Items *)

item        ::= function | struct_def | enum_def | trait_def
              | impl_block | module | use_decl

function    ::= "fn" ident generics? "(" params? ")" ("->" type)? (block | ";")
              (* a ";" body declares an ambient function — see Ambient Modules *)
params      ::= param ("," param)* ","?
param       ::= "mut"? pattern (":" type)?

struct_def  ::= "pub"? "struct" ident generics? struct_body
struct_body ::= "{" field ("," field)* ","? "}"
              | "(" type ("," type)* ","? ")" ";"
              | ";"
field       ::= "pub"? ident (":" type)?

enum_def    ::= "pub"? "enum" ident generics? "{" variant ("," variant)* ","? "}"
variant     ::= ident (tuple_fields | struct_fields)?
tuple_fields  ::= "(" type ("," type)* ")"
struct_fields ::= "{" field ("," field)* "}"

trait_def   ::= "trait" ident generics? (":" bounds)? "{" trait_item* "}"
trait_item  ::= "fn" ident generics? "(" params? ")" ("->" type)? (block | ";")
              | "type" ident ";"

impl_block  ::= "impl" generics? type ("for" type)? "{" (function | assoc_type)* "}"
assoc_type  ::= "type" ident "=" type ";"

module      ::= "mod" ident (";" | "{" item* "}")
use_decl    ::= "use" path ("as" ident)? ";"
              | "use" path "::" "{" use_list "}" ";"
              | "use" path "::" "*" ";"

generics    ::= "<" generic_param ("," generic_param)* ">"
generic_param ::= ident (":" bounds)?
bounds      ::= type ("+" type)*
where_clause  ::= "where" (type ":" bounds ",")*

(* -------------------------------------------------------------- *)
(* Statements *)

statement   ::= let_stmt | item | expr ";"
let_stmt    ::= "pub"? "let" "mut"? pattern (":" type)? ("=" expr)? ";"
              | "let" pattern (":" type)? "=" expr "else" block  (* let-else *)
              (* "pub" is only meaningful at module scope — see
                 Modules and Visibility *)

block       ::= "{" statement* expr? "}"

(* -------------------------------------------------------------- *)
(* Expressions *)

expr        ::= literal | ident | unary_expr | binary_expr
              | call_expr | field_expr | index_expr
              | if_expr | match_expr | loop_expr | while_expr | for_expr
              | block | closure | struct_expr | tuple_expr | array_expr
              | "return" expr? | "break" ident? expr? | "continue" ident?
              | expr "?"

if_expr     ::= "if" expr block ("else" (if_expr | if_let_expr | block))?
if_let_expr ::= "if" "let" pattern "=" expr block ("else" ...)?
match_expr  ::= "match" expr "{" match_arm ("," match_arm)* ","? "}"
match_arm   ::= pattern ("|" pattern)* ("if" expr)? "=>" expr

label       ::= ident ":"
loop_expr   ::= label? "loop" block
while_expr  ::= label? "while" expr block
              | label? "while" "let" pattern "=" expr block
for_expr    ::= label? "for" pattern "in" expr block

closure     ::= "|" params? "|" ("->" type)? (block | expr)

struct_expr ::= path "{" field_init ("," field_init)* ","? (".." expr)? "}"
field_init  ::= ident (":" expr)?
tuple_expr  ::= "(" (expr ",")+ expr? ")"
array_expr  ::= "[" (expr ("," expr)* ","?)? "]"

call_expr   ::= expr "(" (expr ("," expr)*)? ")"
field_expr  ::= expr "." (ident | int_literal)
index_expr  ::= expr "[" expr "]"

(* -------------------------------------------------------------- *)
(* Patterns *)

pattern     ::= "_" | ident ("@" pattern)? | literal
              | range_pattern | tuple_pattern | array_pattern
              | struct_pattern | path tuple_pattern? | pattern "|" pattern

range_pattern  ::= expr (".." | "..=") expr
tuple_pattern  ::= "(" (pattern ",")* pattern? ")"
array_pattern  ::= "[" (pattern ("," pattern)* (",", "..")?)? "]"
struct_pattern ::= path "{" (field_pattern ("," field_pattern)*)? (",", "..")? "}"
field_pattern  ::= ident (":" pattern)?

(* -------------------------------------------------------------- *)
(* Types *)

type        ::= path generic_args?             (* named/generic type, incl.
                                                    trait-as-type; also how
                                                    every builtin type name —
                                                    bool/int/float/char/
                                                    String/any — parses,
                                                    since none of them are
                                                    keywords *)
              | "(" (type ",")* type? ")"       (* tuple / unit *)
              | "[" type "]"                    (* array *)
              | "Fn" "(" (type ("," type)*)? ")" ("->" type)?
generic_args  ::= "<" type ("," type)* ">"
path        ::= ident ("::" ident)*

(* -------------------------------------------------------------- *)
(* Lexical *)

literal     ::= int_literal | float_literal | string_literal
              | char_literal | "true" | "false"
ident       ::= (letter | "_") (letter | digit | "_")*
```

See [Differences from Rust](../design/differences-from-rust.md) for
everything deliberately absent from this grammar (references, lifetimes,
`unsafe`, macros, attributes, and the rest).
