use super::*;
use std::collections::HashSet;
use std::ops::Range;

use ast::{Block, Expr, ExprKind, Local, Pat, StmtKind};
use chumsky::Parser;
use unify::Term;

use crate::call_graph::{
    AdjacencyListGraph, CallGraph, CallGraphCollector, strongly_connected_components,
};

fn resolve(source: &str) -> TypeCheckContext {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let items = parser::module()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse");

    let mut cx = TypeCheckContext::new();
    cx.resolve(&items);
    cx
}

fn resolve_and_lower(source: &str) -> TypeCheckContext {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let items = parser::module()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse");

    let mut cx = TypeCheckContext::new();
    cx.resolve(&items);
    cx.lower_signatures(&items);
    cx
}

fn lookup(cx: &TypeCheckContext, scope: ScopeId, namespace: Namespace, name: &str) -> bool {
    let Some(&name) = cx.names.ids.get(name) else {
        return false;
    };
    let map = match namespace {
        Namespace::Type => &cx.scopes[scope].types,
        Namespace::Value => &cx.scopes[scope].values,
    };
    map.contains_key(&name)
}

fn path(segments: &[&str]) -> Path {
    let dummy_span = ast::Span { start: 0, end: 0 };
    Path {
        segments: segments
            .iter()
            .map(|name| ast::PathSegment {
                ident: ast::Ident {
                    name: name.to_string(),
                    span: dummy_span,
                },
                args: None,
            })
            .collect(),
        span: dummy_span,
    }
}

fn expr(source: &str) -> Expr {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    parser::expr()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse")
}

fn ty(source: &str) -> Ty {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    parser::ty()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse")
}

fn pat(source: &str) -> Pat {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    parser::pat(parser::expr())
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse")
}

fn block(source: &str) -> Block {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    parser::block(parser::expr())
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse")
}

fn local(source: &str) -> Local {
    let mut blk = block(&format!("{{ {source} }}"));
    let stmt = blk.stmts.remove(0);
    let StmtKind::Let(local) = stmt.kind else {
        panic!("expected a let statement, got {:?}", stmt.kind);
    };
    *local
}

fn resolved_args(cx: &mut TypeCheckContext, term: TermId) -> Option<(TyCon, Vec<TermId>)> {
    let resolved = cx.uni_cx.resolve(term);
    match cx.uni_cx.term(resolved) {
        Some(Term::App { constructor, args }) => Some((constructor.clone(), args.clone())),
        _ => None,
    }
}

fn resolved_con(cx: &mut TypeCheckContext, term: TermId) -> Option<TyCon> {
    resolved_args(cx, term).map(|(con, _)| con)
}

fn declared_symbol(
    cx: &TypeCheckContext,
    scope: ScopeId,
    namespace: Namespace,
    name: &str,
) -> Option<SymbolId> {
    let name = *cx.names.ids.get(name)?;
    let map = match namespace {
        Namespace::Type => &cx.scopes[scope].types,
        Namespace::Value => &cx.scopes[scope].values,
    };
    map.get(&name).copied()
}

#[test]
fn declares_a_free_fn_in_the_value_namespace() {
    let cx = resolve("fn bar() {}");
    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "bar"));
}

#[test]
fn declares_a_named_struct_only_in_the_type_namespace() {
    let cx = resolve("struct Foo { x: int }");
    assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
    assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
}

#[test]
fn declares_a_tuple_struct_in_both_namespaces() {
    let cx = resolve("struct Foo(int);");
    assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
}

#[test]
fn a_struct_and_a_fn_can_share_a_name() {
    let cx = resolve("struct Foo { x: int } fn Foo() {}");
    assert!(lookup(&cx, cx.current_scope, Namespace::Type, "Foo"));
    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "Foo"));
}

#[test]
fn a_mod_gets_its_own_child_scope() {
    let cx = resolve("mod m { fn baz() {} }");
    assert!(lookup(&cx, cx.current_scope, Namespace::Type, "m"));
    assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "baz"));

    let child_scope = cx
        .scopes
        .iter()
        .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
        .expect("mod should have created a child scope");
    assert!(lookup(&cx, child_scope, Namespace::Value, "baz"));
}

#[test]
fn an_item_nested_inside_a_fn_body_is_hoisted_into_its_own_scope() {
    let cx = resolve("fn outer() { fn inner() {} }");
    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "outer"));
    assert!(!lookup(&cx, cx.current_scope, Namespace::Value, "inner"));

    let body_scope = cx
        .scopes
        .iter()
        .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
        .expect("the fn body should have created a child scope");
    assert!(lookup(&cx, body_scope, Namespace::Value, "inner"));
}

#[test]
fn resolve_path_finds_a_single_segment_name() {
    let mut cx = resolve("struct Foo { x: int }");
    assert!(cx.resolve_path(&path(&["Foo"]), Namespace::Type).is_some());
}

#[test]
fn resolve_path_fails_on_an_undeclared_name() {
    let mut cx = resolve("struct Foo { x: int }");
    assert!(cx.resolve_path(&path(&["Bar"]), Namespace::Type).is_none());
}

#[test]
fn resolve_path_checks_the_requested_namespace() {
    let mut cx = resolve("struct Foo { x: int }");
    assert!(cx.resolve_path(&path(&["Foo"]), Namespace::Value).is_none());
}

#[test]
fn resolve_path_walks_through_a_module() {
    let mut cx = resolve("mod m { fn baz() {} }");
    let resolved = cx.resolve_path(&path(&["m", "baz"]), Namespace::Value);
    assert!(resolved.is_some());
}

#[test]
fn resolve_path_rejects_walking_through_a_non_module_segment() {
    let mut cx = resolve("struct Foo { x: int } fn bar() {}");
    assert!(
        cx.resolve_path(&path(&["Foo", "bar"]), Namespace::Value)
            .is_none()
    );
}

#[test]
fn resolve_path_module_segment_is_looked_up_by_namespace_not_by_name_alone() {
    let mut cx = resolve("mod m { fn baz() {} } fn m() {}");
    assert!(
        cx.resolve_path(&path(&["m", "baz"]), Namespace::Value)
            .is_some()
    );
}

#[test]
fn lower_ty_never() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("!"));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
}

#[test]
fn lower_ty_paren_unwraps_to_the_inner_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("(!)"));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
}

#[test]
fn lower_ty_array_wraps_the_element_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("[!]"));
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Array);
    assert_eq!(args.len(), 1);
    assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Never));
}

#[test]
fn lower_ty_tup_builds_one_arg_per_element() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("(!, !)"));
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Tuple);
    assert_eq!(args.len(), 2);
}

#[test]
fn lower_ty_unit_is_a_zero_arg_tuple() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("()"));
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Tuple);
    assert!(args.is_empty());
}

#[test]
fn lower_ty_fn_with_explicit_return_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("Fn(!) -> !"));
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Fn);
    assert_eq!(args.len(), 2);
    assert_eq!(resolved_con(&mut cx, args[1]), Some(TyCon::Never));
}

#[test]
fn lower_ty_fn_with_no_return_type_defaults_to_a_fresh_unbound_var() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("Fn(!)"));
    let (_, args) = resolved_args(&mut cx, t).expect("should be an App term");
    let resolved = cx.uni_cx.resolve(args[1]);
    assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
}

#[test]
fn lower_ty_infer_produces_a_fresh_unbound_var() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("_"));
    let resolved = cx.uni_cx.resolve(t);
    assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
}

#[test]
fn lower_ty_err_is_a_wildcard_that_unifies_with_anything() {
    let mut cx = TypeCheckContext::new();
    let err_ty = Ty {
        kind: TyKind::Err,
        span: ast::Span { start: 0, end: 0 },
    };
    let err_term = cx.lower_ty(&err_ty);
    let int_term = term!(cx.uni_cx, TyCon::Int);
    assert!(cx.uni_cx.unify(err_term, int_term).is_ok());
}

#[test]
fn lower_ty_path_resolves_primitive_names() {
    let cases = [
        ("bool", TyCon::Bool),
        ("int", TyCon::Int),
        ("float", TyCon::Float),
        ("char", TyCon::Char),
        ("String", TyCon::Str),
    ];
    for (src, expected) in cases {
        let mut cx = TypeCheckContext::new();
        let t = cx.lower_ty(&ty(src));
        assert_eq!(resolved_con(&mut cx, t), Some(expected), "input: {src}");
    }
}

#[test]
fn lower_ty_path_resolves_a_declared_struct_by_nominal_identity() {
    let mut cx = resolve("struct Foo { x: int }");
    let symbol = cx
        .resolve_path(&path(&["Foo"]), Namespace::Type)
        .expect("Foo should resolve");

    let t = cx.lower_ty(&ty("Foo"));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Struct(symbol)));
}

#[test]
fn lower_ty_path_resolves_a_declared_enum_by_nominal_identity() {
    let mut cx = resolve("enum Foo { Bar }");
    let symbol = cx
        .resolve_path(&path(&["Foo"]), Namespace::Type)
        .expect("Foo should resolve");

    let t = cx.lower_ty(&ty("Foo"));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Enum(symbol)));
}

#[test]
fn lower_ty_path_to_an_undeclared_name_is_err() {
    let mut cx = TypeCheckContext::new();
    let t = cx.lower_ty(&ty("DoesNotExist"));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Err));
}

#[test]
fn lower_ty_path_walks_through_a_module() {
    let mut cx = resolve("mod m { struct Foo; }");
    let symbol = cx
        .resolve_path(&path(&["m", "Foo"]), Namespace::Type)
        .expect("m::Foo should resolve");

    let t = cx.lower_ty(&ty("m::Foo"));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Struct(symbol)));
}

#[test]
fn check_expr_bool_literal() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("true"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Bool));
    let t = cx.check_expr(&expr("false"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Bool));
}

#[test]
fn check_expr_int_literal() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("5"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn check_expr_float_literal() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("5.0"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Float));
}

#[test]
fn check_expr_str_literal() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("\"hi\""), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Str));
}

#[test]
fn check_expr_char_literal() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("'a'"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Char));
}

#[test]
fn check_expr_paren_has_the_inner_exprs_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("(5)"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn check_expr_err_is_a_wildcard() {
    let mut cx = TypeCheckContext::new();
    let err_expr = Expr {
        annotations: Vec::new(),
        kind: ExprKind::Err,
        span: ast::Span { start: 0, end: 0 },
    };
    let bool_term = term!(cx.uni_cx, TyCon::Bool);
    let t = cx.check_expr(&err_expr, Some(bool_term));
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Err));
}

#[test]
fn check_expr_unifies_the_result_against_the_expected_type() {
    let mut cx = resolve("fn foo() {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    let never_term = term!(cx.uni_cx, TyCon::Never);
    cx.check_expr(&expr("foo"), Some(never_term));

    assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Never));
}

#[test]
fn check_expr_tup_elements_keep_independent_types() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("(1, \"hi\")"), None);
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Tuple);
    assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Int));
    assert_eq!(resolved_con(&mut cx, args[1]), Some(TyCon::Str));
}

#[test]
fn check_expr_array_elements_are_unified_with_each_other() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("[1, 2, 3]"), None);
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Array);
    assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Int));
}

#[test]
fn check_expr_empty_array_uses_the_expected_element_type() {
    let mut cx = TypeCheckContext::new();
    let never_term = term!(cx.uni_cx, TyCon::Never);
    let array_of_never = term!(cx.uni_cx, TyCon::Array => [never_term]);

    let t = cx.check_expr(&expr("[]"), Some(array_of_never));
    let (_, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(resolved_con(&mut cx, args[0]), Some(TyCon::Never));
}

#[test]
fn check_expr_path_resolves_to_the_symbols_type() {
    let mut cx = resolve("fn foo() {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    let t = cx.check_expr(&expr("foo"), None);
    assert_eq!(t, symbol_ty);
}

#[test]
fn check_expr_path_to_an_undeclared_name_is_err() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("doesNotExist"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Err));
}

#[test]
fn check_expr_cast_lowers_the_target_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("5 as float"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Float));
}

#[test]
fn check_expr_call_pins_the_callees_type_to_a_fn_shape() {
    let mut cx = resolve("fn foo() {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    cx.check_expr(&expr("foo()"), None);

    let (con, _) = resolved_args(&mut cx, symbol_ty).expect("should be an App term");
    assert_eq!(con, TyCon::Fn);
}

#[test]
fn check_expr_call_checks_arguments_against_the_signature() {
    let mut cx = resolve("fn foo() {}");
    cx.check_expr(&expr("foo(5)"), None);

    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    let (_, fn_args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    let (_, input_args) = resolved_args(&mut cx, fn_args[0]).expect("should be a Tuple term");
    assert_eq!(resolved_con(&mut cx, input_args[0]), Some(TyCon::Int));
}

#[test]
fn check_all_calling_an_annotated_non_fn_parameter_is_an_error() {
    let source = r#"
fn use_it(g: int) {
    g(1);
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.message(), "expected a function, found `int`");
    assert_eq!(&source[d.span().start..d.span().end], "g");
}

#[test]
fn check_all_calling_a_locally_inferred_non_fn_value_is_an_error() {
    let source = r#"
fn use_it() {
    let g = 5;
    g(1);
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected a function, found `int`");
}

#[test]
fn check_all_calling_an_unannotated_parameter_infers_its_fn_shape_with_no_error() {
    let source = r#"
fn apply(f, x) {
    f(x)
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let apply = cx
        .resolve_path(&path(&["apply"]), Namespace::Value)
        .expect("apply should resolve");
    assert_eq!(
        cx.symbols[apply].generics.len(),
        2,
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );
}

#[test]
fn check_all_self_application_is_a_cyclic_type_error() {
    let source = r#"
fn cyclic(x) {
    x(x)
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].level(), Level::Error);
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
}

#[test]
fn check_expr_call_result_is_an_unbound_var_when_nothing_constrains_it() {
    let mut cx = resolve("fn foo() {}");
    let t = cx.check_expr(&expr("foo()"), None);
    let resolved = cx.uni_cx.resolve(t);
    assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
}

#[test]
fn check_expr_ret_with_no_value_is_never() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("return"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
}

#[test]
fn check_expr_ret_with_a_value_is_still_never_not_the_values_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("return 5"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
}

#[test]
fn never_is_a_wildcard_that_unifies_with_anything() {
    let mut cx = TypeCheckContext::new();
    let never_term = term!(cx.uni_cx, TyCon::Never);
    let int_term = term!(cx.uni_cx, TyCon::Int);
    assert!(cx.uni_cx.unify(never_term, int_term).is_ok());
}

#[test]
fn if_with_no_else_and_a_unit_then_branch_is_unit_typed() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("if true { }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Tuple));
}

#[test]
fn if_branches_are_unified_together() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("if true { 1 } else { 2 }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn if_prefers_the_else_branchs_type_when_the_then_branch_diverges() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("if true { return } else { 5 }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("if true { 5 } else { return }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn if_is_never_when_both_branches_diverge() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("if true { return } else { return }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
}

#[test]
fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges_via_a_semicolon() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_expr(&expr("if true { 5 } else { return 0; }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn check_block_empty_is_unit() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_block(&block("{}"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Tuple));
}

#[test]
fn check_block_trailing_expr_with_no_semicolon_is_its_type() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_block(&block("{ 5 }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn check_block_trailing_expr_with_a_semicolon_does_not_count() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_block(&block("{ 5; }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Tuple));
}

#[test]
fn check_block_a_semicolon_terminated_return_makes_the_block_never() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_block(&block("{ return 0; }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Never));
}

#[test]
fn check_block_a_non_trailing_let_declares_a_symbol_visible_to_later_statements() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_block(&block("{ let x = 5; x }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Int));
}

#[test]
fn check_block_a_non_trailing_lets_ascription_propagates_to_a_later_reference() {
    let mut cx = TypeCheckContext::new();
    let t = cx.check_block(&block("{ let x: float; let y = x; y }"), None);
    assert_eq!(resolved_con(&mut cx, t), Some(TyCon::Float));
}

#[test]
fn check_pat_ident_declares_a_local_symbol() {
    let mut cx = TypeCheckContext::new();
    let never_term = term!(cx.uni_cx, TyCon::Never);
    cx.check_pat(&pat("x"), never_term, PatDeclKind::Let);

    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "x"));
}

#[test]
fn check_pat_ident_binds_the_locals_type_to_expected() {
    let mut cx = TypeCheckContext::new();
    let never_term = term!(cx.uni_cx, TyCon::Never);
    cx.check_pat(&pat("x"), never_term, PatDeclKind::Let);

    let symbol = declared_symbol(&cx, cx.current_scope, Namespace::Value, "x")
        .expect("x should be declared");
    let symbol_ty = cx.symbols[symbol].ty;
    assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Never));
}

#[test]
fn check_pat_wild_matches_anything_and_binds_nothing() {
    let mut cx = TypeCheckContext::new();
    let never_term = term!(cx.uni_cx, TyCon::Never);
    let t = cx.check_pat(&pat("_"), never_term, PatDeclKind::Let);
    assert_eq!(t, never_term);
    assert!(cx.symbols.is_empty());
}

#[test]
fn check_pat_tuple_declares_one_local_per_position() {
    let mut cx = TypeCheckContext::new();
    let never_term = term!(cx.uni_cx, TyCon::Never);
    let int_term = term!(cx.uni_cx, TyCon::Int);
    let expected = term!(cx.uni_cx, TyCon::Tuple => [never_term, int_term]);

    cx.check_pat(&pat("(a, b)"), expected, PatDeclKind::Let);

    let a = declared_symbol(&cx, cx.current_scope, Namespace::Value, "a")
        .expect("a should be declared");
    let b = declared_symbol(&cx, cx.current_scope, Namespace::Value, "b")
        .expect("b should be declared");
    let a_ty = cx.symbols[a].ty;
    let b_ty = cx.symbols[b].ty;
    assert_eq!(resolved_con(&mut cx, a_ty), Some(TyCon::Never));
    assert_eq!(resolved_con(&mut cx, b_ty), Some(TyCon::Int));
}

#[test]
fn check_pat_tuple_with_no_matching_expected_shape_uses_fresh_vars_per_position() {
    let mut cx = TypeCheckContext::new();
    let int_term = term!(cx.uni_cx, TyCon::Int);
    let t = cx.check_pat(&pat("(a, b)"), int_term, PatDeclKind::Let);
    let (con, args) = resolved_args(&mut cx, t).expect("should be an App term");
    assert_eq!(con, TyCon::Tuple);
    assert_eq!(args.len(), 2);
}

#[test]
fn check_local_declares_the_pattern_with_the_initializers_type() {
    let mut cx = TypeCheckContext::new();
    cx.check_local(&local("let x = 5;"));

    let symbol = declared_symbol(&cx, cx.current_scope, Namespace::Value, "x")
        .expect("x should be declared");
    let symbol_ty = cx.symbols[symbol].ty;
    assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Int));
}

#[test]
fn check_local_with_no_initializer_uses_the_ascription() {
    let mut cx = TypeCheckContext::new();
    cx.check_local(&local("let x: !;"));

    let symbol = declared_symbol(&cx, cx.current_scope, Namespace::Value, "x")
        .expect("x should be declared");
    let symbol_ty = cx.symbols[symbol].ty;
    assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Never));
}

#[test]
fn check_local_ascription_constrains_the_initializer() {
    let mut cx = resolve("fn foo() {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");

    cx.check_local(&local("let x: ! = foo();"));

    let symbol_ty = cx.symbols[symbol].ty;
    let (_, fn_args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    assert_eq!(resolved_con(&mut cx, fn_args[1]), Some(TyCon::Never));
}

#[test]
fn lower_signatures_fn_with_typed_params_and_return() {
    let mut cx = resolve_and_lower("fn add(a: int, b: int) -> float { a }");
    let symbol = cx
        .resolve_path(&path(&["add"]), Namespace::Value)
        .expect("add should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    let (con, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    assert_eq!(con, TyCon::Fn);
    let (_, input_args) = resolved_args(&mut cx, args[0]).expect("should be a Tuple term");
    assert_eq!(resolved_con(&mut cx, input_args[0]), Some(TyCon::Int));
    assert_eq!(resolved_con(&mut cx, input_args[1]), Some(TyCon::Int));
    assert_eq!(resolved_con(&mut cx, args[1]), Some(TyCon::Float));
}

#[test]
fn lower_signatures_fn_with_no_return_type_is_a_fresh_unbound_var() {
    let mut cx = resolve_and_lower("fn foo() {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    let resolved = cx.uni_cx.resolve(args[1]);
    assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
}

#[test]
fn lower_signatures_fn_with_an_untyped_param_gets_a_fresh_var() {
    let mut cx = resolve_and_lower("fn foo(x) {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    let (_, input_args) = resolved_args(&mut cx, args[0]).expect("should be a Tuple term");
    let resolved = cx.uni_cx.resolve(input_args[0]);
    assert!(matches!(cx.uni_cx.term(resolved), Some(Term::Var(_))));
}

#[test]
fn lower_signatures_ty_alias() {
    let mut cx = resolve_and_lower("type MyInt = int;");
    let symbol = cx
        .resolve_path(&path(&["MyInt"]), Namespace::Type)
        .expect("MyInt should resolve");
    let symbol_ty = cx.symbols[symbol].ty;
    assert_eq!(resolved_con(&mut cx, symbol_ty), Some(TyCon::Int));
}

#[test]
fn lower_signatures_recurses_into_a_fns_own_body() {
    let mut cx = resolve_and_lower("fn outer() { fn inner(x: int) -> bool { true } }");
    let body_scope = cx
        .scopes
        .iter()
        .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
        .expect("outer's body should have a child scope");
    let symbol = declared_symbol(&cx, body_scope, Namespace::Value, "inner")
        .expect("inner should be declared");
    let symbol_ty = cx.symbols[symbol].ty;
    let (con, _) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    assert_eq!(con, TyCon::Fn);
}

#[test]
fn lower_signatures_recurses_into_a_mod() {
    let mut cx = resolve_and_lower("mod m { fn baz(x: bool) {} }");
    let m_symbol = cx
        .resolve_path(&path(&["m"]), Namespace::Type)
        .expect("m should resolve");
    let SymbolKind::Mod(m_scope) = &cx.symbols[m_symbol].kind else {
        panic!("m should be a Mod symbol");
    };
    let m_scope = *m_scope;

    let symbol =
        declared_symbol(&cx, m_scope, Namespace::Value, "baz").expect("baz should resolve");
    let symbol_ty = cx.symbols[symbol].ty;
    let (con, _) = resolved_args(&mut cx, symbol_ty).expect("should be a Fn term");
    assert_eq!(con, TyCon::Fn);
}

#[test]
fn lower_signatures_makes_the_declared_signature_authoritative() {
    let mut cx = resolve_and_lower("fn foo(x: int) {}");
    let symbol = cx
        .resolve_path(&path(&["foo"]), Namespace::Value)
        .expect("foo should resolve");
    let symbol_ty = cx.symbols[symbol].ty;

    cx.check_expr(&expr("foo(\"wrong\")"), None);

    let (_, args) = resolved_args(&mut cx, symbol_ty).expect("should still be a Fn term");
    let (_, input_args) = resolved_args(&mut cx, args[0]).expect("should still be a Tuple term");
    assert_eq!(resolved_con(&mut cx, input_args[0]), Some(TyCon::Int));
}

fn check_all(source: &str) -> TypeCheckContext {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let items = parser::module()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse");

    let mut cx = TypeCheckContext::new();
    cx.resolve(&items);
    cx.lower_signatures(&items);
    cx.check(&items);
    cx
}

fn fn_body_scope(cx: &TypeCheckContext, symbol: SymbolId) -> ScopeId {
    match &cx.symbols[symbol].kind {
        SymbolKind::Fn(fn_data) => fn_data.scope,
        _ => panic!("expected a Fn symbol"),
    }
}

fn generics_list(generic_names: &GenericNames, generics: &[GenericId]) -> String {
    if generics.is_empty() {
        return String::new();
    }
    let names: Vec<String> = generics
        .iter()
        .map(|id| {
            generic_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| "<generic>".to_owned())
        })
        .collect();
    format!("<{}>", names.join(", "))
}

struct Renderer<'a> {
    uni_cx: &'a mut UnificationContext<TyCon, Span>,
    symbols: &'a SlotMap<SymbolId, Symbol>,
    names: &'a NameInterner,
    generic_names: &'a GenericNames,
}

impl TypeCheckContext {
    fn renderer(&mut self) -> Renderer<'_> {
        Renderer {
            uni_cx: &mut self.uni_cx,
            symbols: &self.symbols,
            names: &self.names,
            generic_names: &self.generic_names,
        }
    }

    fn render_symbol_type(&mut self, symbol: SymbolId) -> String {
        self.renderer().render_symbol_type(symbol)
    }

    fn describe_symbol(&mut self, symbol: SymbolId, at: usize) -> String {
        self.renderer().describe_symbol(symbol, at)
    }
}

impl Renderer<'_> {
    fn render_term(&mut self, term: TermId) -> String {
        let mut buf = String::new();
        self.render_term_into(&mut buf, term, None);
        buf
    }

    fn render_term_into(
        &mut self,
        buf: &mut String,
        term: TermId,
        highlight: Option<TermId>,
    ) -> Option<Range<usize>> {
        if let Some(highlight) = highlight {
            if self.uni_cx.resolve(term) == self.uni_cx.resolve(highlight) {
                let start = buf.len();
                buf.push_str(&self.render_term(term));
                return Some(start..buf.len());
            }
        }

        let resolved = self.uni_cx.resolve(term);
        let Some(term) = self.uni_cx.term(resolved).cloned() else {
            buf.push_str("<error>");
            return None;
        };

        let (constructor, args) = match term {
            Term::Var(_) => {
                buf.push('_');
                return None;
            }
            Term::App { constructor, args } => (constructor, args),
        };

        match constructor {
            TyCon::Any => {
                buf.push_str("any");
                None
            }
            TyCon::Never => {
                buf.push('!');
                None
            }
            TyCon::Int => {
                buf.push_str("int");
                None
            }
            TyCon::Float => {
                buf.push_str("float");
                None
            }
            TyCon::Bool => {
                buf.push_str("bool");
                None
            }
            TyCon::Char => {
                buf.push_str("char");
                None
            }
            TyCon::Str => {
                buf.push_str("String");
                None
            }
            TyCon::Err => {
                buf.push_str("<error>");
                None
            }
            TyCon::Array => {
                buf.push('[');
                let range = self.render_term_into(buf, args[0], highlight);
                buf.push(']');
                range
            }
            TyCon::Tuple => {
                buf.push('(');
                let mut range = None;
                for (i, &arg) in args.iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    range = range.or(self.render_term_into(buf, arg, highlight));
                }
                buf.push(')');
                range
            }
            TyCon::Fn => {
                buf.push_str("Fn");
                let inputs_range = self.render_term_into(buf, args[0], highlight);
                buf.push_str(" -> ");
                let output_range = self.render_term_into(buf, args[1], highlight);
                inputs_range.or(output_range)
            }
            TyCon::Struct(symbol) | TyCon::Enum(symbol) => {
                let name = self.symbols[symbol].name;
                let text = self
                    .names
                    .name(name)
                    .cloned()
                    .unwrap_or_else(|| "<unknown>".to_owned());
                buf.push_str(&text);
                None
            }
            TyCon::Generic(id) => {
                let text = self
                    .generic_names
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "<generic>".to_owned());
                buf.push_str(&text);
                None
            }
        }
    }

    fn render_symbol_type(&mut self, symbol: SymbolId) -> String {
        let ty = self.symbols[symbol].ty;
        let rendered = self.render_term(ty);
        let generics = self.symbols[symbol].generics.clone();
        let generics_rendered = generics_list(self.generic_names, &generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    fn describe_symbol(&mut self, symbol: SymbolId, at: usize) -> String {
        match &self.symbols[symbol].kind {
            SymbolKind::Fn(_) => self.describe_fn_item(symbol),
            SymbolKind::Param => {
                let name = self.symbol_display_name(symbol);
                let ty = self.render_symbol_type(symbol);
                format!("{name}: {ty}")
            }
            SymbolKind::Local => {
                let name = self.symbol_display_name(symbol);
                let ty = self.render_symbol_type(symbol);
                format!("let {name}: {ty}")
            }
            SymbolKind::TyAlias(_) => {
                let declared_at = self.symbols[symbol].declared_at;
                let rendered = self.alias_name_with_generics(symbol);
                if declared_at.start <= at && at < declared_at.end {
                    format!("type {rendered}")
                } else {
                    rendered
                }
            }
            SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Variant
            | SymbolKind::Trait
            | SymbolKind::Mod(_)
            | SymbolKind::GenericParam => self.render_symbol_type(symbol),
        }
    }

    fn symbol_display_name(&mut self, symbol: SymbolId) -> String {
        let name = self.symbols[symbol].name;
        self.names
            .name(name)
            .cloned()
            .unwrap_or_else(|| "_".to_owned())
    }

    fn alias_name_with_generics(&mut self, symbol: SymbolId) -> String {
        let name = self.symbol_display_name(symbol);
        let generics = self.symbols[symbol].generics.clone();
        let generics_rendered = generics_list(self.generic_names, &generics);
        format!("{name}{generics_rendered}")
    }

    fn describe_fn_item(&mut self, symbol: SymbolId) -> String {
        let name = self.symbols[symbol].name;
        let name = self
            .names
            .name(name)
            .cloned()
            .unwrap_or_else(|| "<unknown>".to_owned());

        let generics = self.symbols[symbol].generics.clone();
        let generics_rendered = generics_list(self.generic_names, &generics);

        let SymbolKind::Fn(FnSymbol { param_names, .. }) = &self.symbols[symbol].kind else {
            unreachable!("describe_fn_item is only ever called for a SymbolKind::Fn symbol");
        };
        let param_names = param_names.clone();

        let ty = self.symbols[symbol].ty;
        let resolved = self.uni_cx.resolve(ty);
        let Some(Term::App {
            constructor: TyCon::Fn,
            args,
        }) = self.uni_cx.term(resolved).cloned()
        else {
            return self.render_symbol_type(symbol);
        };
        let (inputs, output) = (args[0], args[1]);

        let resolved_inputs = self.uni_cx.resolve(inputs);
        let param_types: Vec<TermId> = match self.uni_cx.term(resolved_inputs).cloned() {
            Some(Term::App {
                constructor: TyCon::Tuple,
                args,
            }) => args,
            _ => Vec::new(),
        };

        let params: Vec<String> = param_types
            .iter()
            .enumerate()
            .map(|(i, &ty)| {
                let rendered = self.render_term(ty);
                match param_names.get(i) {
                    Some(name) => format!("{name}: {rendered}"),
                    None => rendered,
                }
            })
            .collect();

        let output_rendered = self.render_term(output);
        format!(
            "fn {name}{generics_rendered}({}) -> {output_rendered}",
            params.join(", ")
        )
    }
}

#[test]
fn check_all_infers_an_untyped_params_type_from_the_bodys_declared_return_type() {
    let mut cx = check_all("fn identity(x) -> int { x }");
    let fn_symbol = cx
        .resolve_path(&path(&["identity"]), Namespace::Value)
        .expect("identity should resolve");
    let body_scope = fn_body_scope(&cx, fn_symbol);

    let x_symbol = declared_symbol(&cx, body_scope, Namespace::Value, "x")
        .expect("x should be declared as a param");
    let x_ty = cx.symbols[x_symbol].ty;
    assert_eq!(resolved_con(&mut cx, x_ty), Some(TyCon::Int));
}

#[test]
fn check_all_recurses_into_a_nested_fns_body() {
    let mut cx = check_all("fn outer() { fn inner(x) -> int { x } }");
    let outer_symbol = cx
        .resolve_path(&path(&["outer"]), Namespace::Value)
        .expect("outer should resolve");
    let outer_scope = fn_body_scope(&cx, outer_symbol);

    let inner_symbol = declared_symbol(&cx, outer_scope, Namespace::Value, "inner")
        .expect("inner should be declared inside outer's body");
    let inner_scope = fn_body_scope(&cx, inner_symbol);

    let x_symbol = declared_symbol(&cx, inner_scope, Namespace::Value, "x")
        .expect("x should be declared as inner's param");
    let x_ty = cx.symbols[x_symbol].ty;
    assert_eq!(resolved_con(&mut cx, x_ty), Some(TyCon::Int));
}

#[test]
fn check_all_nested_fn_body_resolves_a_reference_to_an_outer_params_symbol() {
    let source = r#"
fn outer(x: int) {
    fn inner() {
        x;
    }
}
"#;
    let cx = check_all(source);

    let param_decl_offset = source.find("x: int").unwrap();
    let param_use_offset = source.rfind('x').unwrap();
    assert_ne!(param_decl_offset, param_use_offset);

    let decl_symbol = cx
        .symbol_at(param_decl_offset)
        .expect("should resolve at outer's parameter declaration");
    let use_symbol = cx
        .symbol_at(param_use_offset)
        .expect("inner's reference to x should resolve to outer's parameter");
    assert_eq!(decl_symbol, use_symbol);
}

#[test]
fn check_all_emphasizes_only_the_specific_conflicting_portion_of_a_compound_type() {
    let source = r#"
fn add_one(x: int) -> int {
    x
}
fn use_it() {
    let f: Fn(int) -> String = add_one;
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(
        d.message(),
        "expected `Fn(int) -> String`, found `Fn(int) -> int`"
    );
    assert_eq!(d.emphasis().len(), 2, "{:#?}", d.emphasis());

    let highlighted: Vec<&str> = d
        .emphasis()
        .iter()
        .map(|range| &d.message()[range.clone()])
        .collect();
    assert_eq!(highlighted, vec!["String", "int"]);
}

#[test]
fn check_all_call_reports_the_specific_mismatching_argument_not_the_whole_call() {
    let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add("wrong", 5);
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(&source[d.span().start..d.span().end], "\"wrong\"");
    assert_eq!(d.message(), "expected `int`, found `String`");
}

#[test]
fn check_all_call_mismatch_against_an_annotated_param_points_at_the_annotation() {
    let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add("wrong", 5);
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.related().len(), 1, "{:#?}", d.related());
    let (span, message) = &d.related()[0];
    assert_eq!(&source[span.start..span.end], "int");
    assert_eq!(message, "expected due to this");
}

#[test]
fn check_all_call_mismatch_against_an_unannotated_param_has_no_expected_due_to_this_note() {
    let source = r#"
fn takes_something(x) {
    let y: int = x;
    x
}
fn use_it() {
    takes_something("wrong");
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(
        diagnostics[0]
            .related()
            .iter()
            .all(|(_, message)| message != "expected due to this"),
        "{:#?}",
        diagnostics[0].related()
    );
}

#[test]
fn check_all_call_mismatch_against_an_unannotated_param_cites_where_it_was_inferred() {
    let source = r#"
fn takes_something(x) {
    let y: int = x;
    x
}
fn use_it() {
    takes_something("wrong");
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.related().len(), 1, "{:#?}", d.related());
    let (span, message) = &d.related()[0];
    assert_eq!(&source[span.start..span.end], "int");
    assert_eq!(message, "expected `int` was inferred here");
}

#[test]
fn check_all_cites_where_an_unannotated_params_fn_shape_was_first_inferred() {
    let source = r#"
fn use_it(f) {
    f(1);
    let x: String = f;
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.related().len(), 2, "{:#?}", d.related());

    let expected_note = d
        .related()
        .iter()
        .find(|(_, message)| message == "expected due to this")
        .expect("should still cite the `String` annotation");
    assert_eq!(
        &source[expected_note.0.start..expected_note.0.end],
        "String"
    );

    let provenance_note = d
        .related()
        .iter()
        .find(|(_, message)| message.starts_with("found "))
        .expect("should cite where f's Fn shape was inferred");
    assert_eq!(&source[provenance_note.0.start..provenance_note.0.end], "f");
    assert_eq!(provenance_note.1, "found `Fn(int) -> _` was inferred here");
}

#[test]
fn check_all_a_turbofish_generic_argument_is_cited_as_provenance_on_mismatch() {
    let source = r#"
fn identity<T>(x: T) -> T {
    x
}
fn use_it() {
    identity::<int>("wrong");
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    let provenance_note = d
        .related()
        .iter()
        .find(|(_, message)| message.starts_with("expected `"))
        .expect("should cite where the turbofish argument pinned `T`");
    assert_eq!(
        &source[provenance_note.0.start..provenance_note.0.end],
        "int"
    );
    assert_eq!(provenance_note.1, "expected `int` was inferred here");

    let turbofish_int = source.find("::<int>").unwrap() + "::<".len();
    assert_eq!(provenance_note.0.start, turbofish_int);
}

#[test]
fn symbol_at_finds_a_fns_own_declaration_and_every_later_reference_to_it() {
    let source = r#"
fn add_one(x: int) -> int {
    x
}
fn use_it() {
    add_one(1);
}
"#;
    let mut cx = check_all(source);

    let decl_offset = source.find("add_one").unwrap() + 2;
    let use_offset = source.rfind("add_one").unwrap() + 2;
    assert_ne!(
        decl_offset, use_offset,
        "test source should have two distinct occurrences"
    );

    let decl_symbol = cx
        .symbol_at(decl_offset)
        .expect("should resolve at the declaration");
    let use_symbol = cx
        .symbol_at(use_offset)
        .expect("should resolve at the call site");
    assert_eq!(decl_symbol, use_symbol);
    assert_eq!(cx.render_symbol_type(decl_symbol), "Fn(int) -> int");
}

#[test]
fn symbol_at_finds_a_parameter_at_both_its_declaration_and_its_use_in_the_body() {
    let source = "fn add_one(x: int) -> int { x }";
    let mut cx = check_all(source);

    let param_decl = source.find("x:").unwrap();
    let param_use = source.rfind('x').unwrap();
    assert_ne!(param_decl, param_use);

    let decl_symbol = cx
        .symbol_at(param_decl)
        .expect("should resolve at the parameter");
    let use_symbol = cx
        .symbol_at(param_use)
        .expect("should resolve at the body reference");
    assert_eq!(decl_symbol, use_symbol);
    assert_eq!(cx.render_symbol_type(decl_symbol), "int");
}

#[test]
fn symbol_at_finds_a_let_bindings_inferred_type() {
    let source = r#"
fn use_it() {
    let n = 1;
}
"#;
    let mut cx = check_all(source);
    let offset = source.find("let n").unwrap() + "let ".len();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the let binding");
    assert_eq!(cx.render_symbol_type(symbol), "int");
}

#[test]
fn symbol_at_finds_a_struct_name_at_its_declaration_and_in_a_type_annotation() {
    let source = r#"
struct Point {
    x: int,
}
fn use_it(p: Point) {}
"#;
    let cx = check_all(source);

    let decl_offset = source.find("Point").unwrap();
    let use_offset = source.rfind("Point").unwrap();
    assert_ne!(decl_offset, use_offset);

    let decl_symbol = cx
        .symbol_at(decl_offset)
        .expect("should resolve at the struct decl");
    let use_symbol = cx
        .symbol_at(use_offset)
        .expect("should resolve at the annotation");
    assert_eq!(decl_symbol, use_symbol);
}

#[test]
fn symbol_at_finds_a_generic_param_at_its_declaration_and_in_the_signature() {
    let source = "fn identity<T>(x: T) -> T { x }";
    let cx = check_all(source);

    let decl_offset = source.find('T').unwrap();
    let param_ty_offset = source.find("x: T").unwrap() + 3;
    let ret_ty_offset = source.rfind('T').unwrap();

    let decl_symbol = cx.symbol_at(decl_offset).expect("should resolve at <T>");
    let param_symbol = cx
        .symbol_at(param_ty_offset)
        .expect("should resolve at x: T");
    let ret_symbol = cx.symbol_at(ret_ty_offset).expect("should resolve at -> T");
    assert_eq!(decl_symbol, param_symbol);
    assert_eq!(decl_symbol, ret_symbol);
}

#[test]
fn symbol_at_is_none_between_identifiers() {
    let source = "fn add_one(x: int) -> int { x }";
    let cx = check_all(source);
    let space_offset = source.find(") ->").unwrap();
    assert_eq!(cx.symbol_at(space_offset), None);
}

#[test]
fn describe_symbol_a_fn_item_uses_source_declaration_syntax_with_param_names() {
    let source = "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U { f(g(x)) }";
    let mut cx = check_all(source);
    let offset = source.find("compose").unwrap();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the fn's own name");
    assert_eq!(
        cx.describe_symbol(symbol, offset),
        "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U"
    );
}

#[test]
fn describe_symbol_a_parameter_is_prefixed_with_its_own_name() {
    let source = "fn add_one(x: int) -> int { x }";
    let mut cx = check_all(source);
    let offset = source.find("x:").unwrap();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(cx.describe_symbol(symbol, offset), "x: int");
}

#[test]
fn describe_symbol_a_let_binding_is_prefixed_with_let_and_its_own_name() {
    let source = r#"
fn use_it() {
    let n = 1;
}
"#;
    let mut cx = check_all(source);
    let offset = source.find("let n").unwrap() + "let ".len();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the let binding");
    assert_eq!(cx.describe_symbol(symbol, offset), "let n: int");
}

#[test]
fn describe_symbol_a_higher_order_parameter_keeps_the_bare_fn_type_syntax() {
    let source = "fn apply<T, U>(f: Fn(T) -> U, x: T) -> U { f(x) }";
    let mut cx = check_all(source);
    let offset = source.find("f:").unwrap();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(cx.describe_symbol(symbol, offset), "f: Fn(T) -> U");
}

#[test]
fn describe_symbol_a_ty_alias_declaration_shows_the_type_keyword() {
    let source = "type Pair<T, U> = (T, U);";
    let mut cx = check_all(source);
    let offset = source.find("Pair").unwrap();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the alias's own name");
    assert_eq!(cx.describe_symbol(symbol, offset), "type Pair<T, U>");
}

#[test]
fn describe_symbol_a_ty_alias_reference_omits_the_type_keyword_and_the_expansion() {
    let source = r#"
type Pair<T, U> = (T, U);
fn make_pair<T, U>(a: T, b: U) -> Pair<T, U> {
    (a, b)
}
"#;
    let mut cx = check_all(source);
    let offset = source.rfind("Pair").unwrap();
    let symbol = cx
        .symbol_at(offset)
        .expect("should resolve at the return-type reference");
    assert_eq!(cx.describe_symbol(symbol, offset), "Pair<T, U>");
}

#[test]
fn type_name_at_finds_every_literal_kinds_own_type() {
    let source = r#"
fn use_it() {
    let a = 1;
    let b = 1.5;
    let c = 'x';
    let d = true;
    let e = "hi";
}
"#;
    let cx = check_all(source);
    assert_eq!(cx.type_name_at(source.find('1').unwrap()), Some("int"));
    assert_eq!(cx.type_name_at(source.find("1.5").unwrap()), Some("float"));
    assert_eq!(cx.type_name_at(source.find("'x'").unwrap()), Some("char"));
    assert_eq!(cx.type_name_at(source.find("true").unwrap()), Some("bool"));
    assert_eq!(
        cx.type_name_at(source.find("\"hi\"").unwrap()),
        Some("String")
    );
}

#[test]
fn type_name_at_finds_a_primitive_name_in_an_ordinary_type_annotation() {
    let source = "fn add_one(x: int) -> int { x }";
    let cx = check_all(source);
    let offset = source.find("int").unwrap();
    assert_eq!(cx.type_name_at(offset), Some("int"));
}

#[test]
fn type_name_at_finds_a_primitive_generic_argument_in_a_turbofish() {
    let source = r#"
fn identity<T>(x: T) -> T {
    x
}
fn use_it() {
    identity::<int>(1);
}
"#;
    let cx = check_all(source);
    let offset = source.find("::<int>").unwrap() + "::<".len();
    assert_eq!(cx.type_name_at(offset), Some("int"));
}

#[test]
fn type_name_at_is_none_away_from_any_literal_or_primitive_name() {
    let source = "fn add_one(x: int) -> int { x }";
    let cx = check_all(source);
    let offset = source.find("add_one").unwrap();
    assert_eq!(cx.type_name_at(offset), None);
}

#[test]
fn check_all_call_keeps_checking_later_arguments_after_an_earlier_one_mismatches() {
    let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add("wrong1", "wrong2");
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");

    assert_eq!(
        &source[diagnostics[0].span().start..diagnostics[0].span().end],
        "\"wrong1\""
    );
    assert_eq!(
        &source[diagnostics[1].span().start..diagnostics[1].span().end],
        "\"wrong2\""
    );
}

#[test]
fn check_all_call_reports_too_few_arguments() {
    let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add(1);
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(
        d.message(),
        "this function takes 2 arguments but 1 argument was supplied"
    );
    assert_eq!(&source[d.span().start..d.span().end], "add(1");
}

#[test]
fn check_all_call_reports_too_many_arguments_pointing_at_the_extra_ones() {
    let source = r#"
fn add(a: int, b: int) {}
fn main() {
    add(1, 2, 3);
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(
        d.message(),
        "this function takes 2 arguments but 3 arguments were supplied"
    );
    assert_eq!(&source[d.span().start..d.span().end], "3");
}

fn symbol_ids(n: usize) -> Vec<SymbolId> {
    let mut map: SlotMap<SymbolId, ()> = SlotMap::with_key();
    (0..n).map(|_| map.insert(())).collect()
}

#[test]
fn scc_a_chain_with_no_cycles_is_all_singletons_in_dependency_order() {
    let ids = symbol_ids(3);
    let (a, b, c) = (ids[0], ids[1], ids[2]);

    let mut graph = CallGraph::new();
    graph.call(a, b);
    graph.call(b, c);

    let sccs = strongly_connected_components(&graph);

    assert_eq!(sccs, vec![vec![c], vec![b], vec![a]]);
}

#[test]
fn scc_a_two_cycle_is_one_component() {
    let ids = symbol_ids(2);
    let (a, b) = (ids[0], ids[1]);

    let mut graph = CallGraph::new();
    graph.call(a, b);
    graph.call(b, a);

    let sccs = strongly_connected_components(&graph);

    assert_eq!(sccs.len(), 1, "{sccs:?}");
    assert_eq!(sccs[0].len(), 2);
    assert!(sccs[0].contains(&a));
    assert!(sccs[0].contains(&b));
}

#[test]
fn scc_three_way_cycle_is_one_component_that_pops_before_an_unrelated_caller() {
    let ids = symbol_ids(4);
    let (a, b, c, caller) = (ids[0], ids[1], ids[2], ids[3]);

    let mut graph = CallGraph::new();
    graph.call(a, b);
    graph.call(b, c);
    graph.call(c, a);
    graph.call(caller, a);

    let sccs = strongly_connected_components(&graph);

    assert_eq!(sccs.len(), 2, "{sccs:?}");
    assert_eq!(sccs[0].len(), 3);
    for member in [a, b, c] {
        assert!(sccs[0].contains(&member));
    }
    assert_eq!(sccs[1], vec![caller]);
}

#[test]
fn check_all_mutually_recursive_siblings_generalize_together_and_stay_reusable() {
    let source = r#"
fn ping(x) {
    if true { x } else { pong(x) }
}
fn pong(y) {
    if true { y } else { ping(y) }
}
fn use_both() {
    ping(1);
    pong("hi");
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let ping = cx
        .resolve_path(&path(&["ping"]), Namespace::Value)
        .expect("ping should resolve");
    let pong = cx
        .resolve_path(&path(&["pong"]), Namespace::Value)
        .expect("pong should resolve");
    assert_eq!(cx.symbols[ping].generics.len(), 1);
    assert_eq!(cx.symbols[pong].generics.len(), 1);
    assert_eq!(cx.symbols[ping].generics[0], cx.symbols[pong].generics[0]);
}

#[test]
fn check_all_a_one_directional_sibling_call_is_not_treated_as_a_cycle() {
    let source = r#"
fn helper(x) {
    x
}
fn caller(y) {
    helper(y)
}
fn use_it() {
    caller(1);
    caller("hi");
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let helper = cx
        .resolve_path(&path(&["helper"]), Namespace::Value)
        .expect("helper should resolve");
    let caller = cx
        .resolve_path(&path(&["caller"]), Namespace::Value)
        .expect("caller should resolve");
    assert_eq!(cx.symbols[helper].generics.len(), 1);
    assert_eq!(cx.symbols[caller].generics.len(), 1);
}

#[test]
fn check_all_a_parameter_shadowing_a_siblings_name_is_not_treated_as_a_call_to_it() {
    let source = r#"
fn apply(f, x) {
    f(x)
}
fn f(x) {
    apply(x, x)
}
"#;
    let mut cx = check_all(source);

    let apply = cx
        .resolve_path(&path(&["apply"]), Namespace::Value)
        .expect("apply should resolve");
    assert_eq!(
        cx.render_symbol_type(apply),
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );

    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
    let call_span = source.rfind("apply(x, x)").unwrap();
    assert!(
        diagnostics[0].span().start >= call_span,
        "expected the diagnostic on f's own `apply(x, x)` call, not apply's declaration: {:#?}",
        diagnostics[0]
    );
}

#[test]
fn check_all_a_let_binding_shadowing_a_siblings_name_is_not_treated_as_a_call_to_it() {
    let source = r#"
fn apply(g, x) {
    let f = g;
    f(x)
}
fn f(x) {
    apply(x, x)
}
"#;
    let mut cx = check_all(source);

    let apply = cx
        .resolve_path(&path(&["apply"]), Namespace::Value)
        .expect("apply should resolve");
    assert_eq!(
        cx.render_symbol_type(apply),
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );

    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
    let call_span = source.rfind("apply(x, x)").unwrap();
    assert!(
        diagnostics[0].span().start >= call_span,
        "expected the diagnostic on f's own `apply(x, x)` call, not apply's declaration: {:#?}",
        diagnostics[0]
    );
}

#[test]
fn call_graph_collector_does_not_descend_into_a_nested_items_own_body() {
    let block = block("{ fn inner() { sibling() } }");

    let mut cx = TypeCheckContext::new();
    let scope = cx.current_scope;
    let dummy_span = Span { start: 0, end: 0 };
    cx.declare(
        "sibling",
        dummy_span,
        SymbolKind::Fn(FnSymbol {
            scope,
            param_spans: Vec::new(),
            param_names: Vec::new(),
        }),
    );
    let outer = cx.declare(
        "outer",
        dummy_span,
        SymbolKind::Fn(FnSymbol {
            scope,
            param_spans: Vec::new(),
            param_names: Vec::new(),
        }),
    );

    let mut graph = CallGraph::new();
    let mut collector = CallGraphCollector::new(outer, &mut graph, &mut cx);
    collector.visit_block(&block);

    assert!(
        graph.edges().is_empty(),
        "a nested item's own call shouldn't be attributed to its enclosing fn: {:?}",
        graph.edges()
    );
}

#[test]
fn check_all_self_recursive_function_generalizes_and_stays_reusable() {
    let source = r#"
fn identity_rec(x) {
    if true { x } else { identity_rec(x) }
}
fn use_it() {
    identity_rec(1);
    identity_rec("hi");
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let identity_rec = cx
        .resolve_path(&path(&["identity_rec"]), Namespace::Value)
        .expect("identity_rec should resolve");
    assert_eq!(cx.symbols[identity_rec].generics.len(), 1);
}

#[test]
fn check_all_a_three_way_cycle_generalizes_together() {
    let source = r#"
fn a(x) {
    if true { x } else { b(x) }
}
fn b(y) {
    if true { y } else { c(y) }
}
fn c(z) {
    if true { z } else { a(z) }
}
fn use_it() {
    a(1);
    b("hi");
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    for name in ["a", "b", "c"] {
        let symbol = cx
            .resolve_path(&path(&[name]), Namespace::Value)
            .unwrap_or_else(|| panic!("{name} should resolve"));
        assert_eq!(cx.symbols[symbol].generics.len(), 1, "{name}");
    }
}

#[test]
fn check_all_a_fully_annotated_cycle_has_nothing_left_to_generalize() {
    let source = r#"
fn ping2(x: int) -> int {
    pong2(x)
}
fn pong2(y: int) -> int {
    ping2(y)
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let ping2 = cx
        .resolve_path(&path(&["ping2"]), Namespace::Value)
        .expect("ping2 should resolve");
    let pong2 = cx
        .resolve_path(&path(&["pong2"]), Namespace::Value)
        .expect("pong2 should resolve");
    assert_eq!(cx.symbols[ping2].generics.len(), 0);
    assert_eq!(cx.symbols[pong2].generics.len(), 0);
}

#[test]
fn check_all_a_newly_generalized_param_never_reuses_an_explicit_generics_name() {
    let source = r#"
fn compose<T>(f, g: Fn(int) -> _, x) -> Fn(T) -> String {
    f(g(x))
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let compose = cx
        .resolve_path(&path(&["compose"]), Namespace::Value)
        .expect("compose should resolve");
    let generics = cx.symbols[compose].generics.clone();
    assert_eq!(generics.len(), 2, "{generics:?}");

    let explicit = generics[0];
    let inferred = generics[1];
    assert_ne!(explicit, inferred, "should be two distinct GenericIds");

    let explicit_name = cx.generic_names.get(&explicit).cloned();
    let inferred_name = cx.generic_names.get(&inferred).cloned();
    assert_eq!(explicit_name.as_deref(), Some("T"));
    assert_ne!(
        explicit_name, inferred_name,
        "the newly-generalized parameter must not render under the \
             same name as the explicit `<T>`"
    );
}

#[test]
fn check_all_a_real_type_error_inside_a_cyclic_group_is_still_reported() {
    let source = r#"
fn ping3(x: int) {
    pong3(x)
}
fn pong3(y: int) {
    ping3("wrong")
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `String`");
}

#[test]
fn check_all_nested_mutually_recursive_fns_generalize_together() {
    let source = r#"
fn outer() {
    fn ping(x) {
        if true { x } else { pong(x) }
    }
    fn pong(y) {
        if true { y } else { ping(y) }
    }
    fn use_both() {
        ping(1);
        pong("hi");
    }
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let outer = cx
        .resolve_path(&path(&["outer"]), Namespace::Value)
        .expect("outer should resolve");
    let outer_scope = fn_body_scope(&cx, outer);

    let ping = declared_symbol(&cx, outer_scope, Namespace::Value, "ping")
        .expect("ping should be declared inside outer's body");
    let pong = declared_symbol(&cx, outer_scope, Namespace::Value, "pong")
        .expect("pong should be declared inside outer's body");
    assert_eq!(cx.symbols[ping].generics.len(), 1);
    assert_eq!(cx.symbols[pong].generics.len(), 1);
}

#[test]
fn check_all_a_nested_fn_never_generalizes_a_variable_free_in_an_enclosing_signature() {
    let source = r#"
fn compose(f) {
    fn inner(g) {
        fn innermost(x) {
            f(g(x))
        }
        innermost
    }
    inner
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let compose = cx
        .resolve_path(&path(&["compose"]), Namespace::Value)
        .expect("compose should resolve");
    let compose_scope = fn_body_scope(&cx, compose);

    let inner = declared_symbol(&cx, compose_scope, Namespace::Value, "inner")
        .expect("inner should be declared inside compose's body");
    let inner_scope = fn_body_scope(&cx, inner);
    let innermost = declared_symbol(&cx, inner_scope, Namespace::Value, "innermost")
        .expect("innermost should be declared inside inner's body");

    assert_eq!(
        cx.symbols[compose].generics.len(),
        3,
        "{:#?}",
        cx.symbols[compose].generics
    );
    // `inner` legitimately generalizes 1 variable of its own: the type shared
    // between `innermost`'s parameter `x` and `inner`'s own parameter `g`'s
    // domain. That variable is not free in the enclosing signature (`f`'s
    // domain/codomain never mention it), so under standard let-polymorphism
    // it's sound for `inner` to generalize it rather than deferring to
    // `compose`. The other 2 variables that stay free at this point (`f`'s
    // domain and codomain) are correctly excluded, and end up on `compose`
    // instead -- which is what this test is actually asserting.
    assert_eq!(cx.symbols[inner].generics.len(), 1);
    assert_eq!(cx.symbols[innermost].generics.len(), 0);
}

#[test]
fn check_all_a_nested_fns_deferred_generalization_is_actually_usable_at_two_types() {
    let source = r#"
fn compose(f) {
    fn inner(g) {
        fn innermost(x) {
            f(g(x))
        }
        innermost
    }
    inner
}
fn int_to_string(n: int) -> String {
    "hi"
}
fn bool_to_int(b: bool) -> int {
    1
}
fn use_it() {
    compose(int_to_string);
    compose(bool_to_int);
}
"#;
    let cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());
}

fn check_all_frozen(source: &str) -> CheckedProgram {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let items = parser::module()
        .parse(parser::input(tokens))
        .into_result()
        .expect("should parse");
    CheckedProgram::check(&items)
}

#[test]
fn freeze_diagnostics_survive_the_freeze_unchanged() {
    let source = "fn use_it(g: int) { g(1); }";
    let frozen = check_all_frozen(source);
    let diagnostics = frozen.diagnostics(Locale::EnUs);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected a function, found `int`");
}

#[test]
fn freeze_render_symbol_type_a_generic_fn() {
    let source = "fn identity<T>(x: T) -> T { x }";
    let frozen = check_all_frozen(source);
    let offset = source.find("identity").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the fn's own name");
    assert_eq!(frozen.render_symbol_type(symbol), "<T> Fn(T) -> T");
}

#[test]
fn freeze_render_symbol_type_a_struct_parameter() {
    let source = r#"
struct Point {
    x: int,
}
fn use_it(p: Point) {}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.find("p: Point").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(frozen.render_symbol_type(symbol), "Point");
}

#[test]
fn freeze_render_symbol_type_a_mutually_recursive_pair_generalized_together() {
    let source = r#"
fn apply(f, x) {
    f(x)
}
fn f(x) {
    apply(x, x)
}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.find("apply").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at apply's own name");
    assert_eq!(
        frozen.render_symbol_type(symbol),
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );
}

#[test]
fn freeze_describe_symbol_a_fn_item_uses_source_declaration_syntax_with_param_names() {
    let source = "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U { f(g(x)) }";
    let frozen = check_all_frozen(source);
    let offset = source.find("compose").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the fn's own name");
    assert_eq!(
        frozen.describe_symbol(symbol, offset),
        "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U"
    );
}

#[test]
fn freeze_describe_symbol_a_parameter_is_prefixed_with_its_own_name() {
    let source = "fn add_one(x: int) -> int { x }";
    let frozen = check_all_frozen(source);
    let offset = source.find("x:").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(frozen.describe_symbol(symbol, offset), "x: int");
}

#[test]
fn freeze_describe_symbol_a_let_binding_is_prefixed_with_let_and_its_own_name() {
    let source = r#"
fn use_it() {
    let n = 1;
}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.find("let n").unwrap() + "let ".len();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the let binding");
    assert_eq!(frozen.describe_symbol(symbol, offset), "let n: int");
}

#[test]
fn freeze_describe_symbol_a_higher_order_parameter_keeps_the_bare_fn_type_syntax() {
    let source = "fn apply<T, U>(f: Fn(T) -> U, x: T) -> U { f(x) }";
    let frozen = check_all_frozen(source);
    let offset = source.find("f:").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(frozen.describe_symbol(symbol, offset), "f: Fn(T) -> U");
}

#[test]
fn freeze_describe_symbol_a_ty_alias_declaration_shows_the_type_keyword() {
    let source = "type Pair<T, U> = (T, U);";
    let frozen = check_all_frozen(source);
    let offset = source.find("Pair").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the alias's own name");
    assert_eq!(frozen.describe_symbol(symbol, offset), "type Pair<T, U>");
}

#[test]
fn freeze_describe_symbol_a_ty_alias_reference_omits_the_type_keyword_and_the_expansion() {
    let source = r#"
type Pair<T, U> = (T, U);
fn make_pair<T, U>(a: T, b: U) -> Pair<T, U> {
    (a, b)
}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.rfind("Pair").unwrap();
    let symbol = frozen
        .symbol_at(offset)
        .expect("should resolve at the return-type reference");
    assert_eq!(frozen.describe_symbol(symbol, offset), "Pair<T, U>");
}

#[test]
fn freeze_type_name_at_still_finds_a_literals_type() {
    let source = "fn use_it() { 1; }";
    let frozen = check_all_frozen(source);
    let offset = source.find('1').unwrap();
    assert_eq!(frozen.type_name_at(offset), Some("int"));
}
