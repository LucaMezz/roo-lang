use super::*;
use std::ops::Range;

use ast::{Block, Expr, ExprKind, Local, Pat, StmtKind};
use chumsky::Parser;
use intern::Interner;

fn resolve<'ast>(source: &str) -> TypeCheckContext<'ast> {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");

    let mut cx = TypeCheckContext::new(state.0);
    cx.resolve(&items);
    cx
}

fn resolve_and_lower<'ast>(source: &str) -> TypeCheckContext<'ast> {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");

    let mut cx = TypeCheckContext::new(state.0);
    cx.resolve(&items);
    cx.lower_signatures(&items);
    cx
}

fn lookup(cx: &TypeCheckContext<'_>, scope: ScopeId, namespace: Namespace, symbol: &str) -> bool {
    let Some(symbol) = cx.symbols.get(symbol) else {
        return false;
    };
    let map = match namespace {
        Namespace::Type => &cx.scopes[scope].types,
        Namespace::Value => &cx.scopes[scope].values,
    };
    map.contains_key(&symbol)
}

fn path(symbols: &mut Interner, segments: &[&str]) -> Path {
    let dummy_span = ast::Span { start: 0, end: 0 };
    Path {
        segments: segments
            .iter()
            .map(|symbol| ast::PathSegment {
                ident: ast::Ident {
                    symbol: symbols.intern(symbol),
                    span: dummy_span,
                },
                args: None,
            })
            .collect(),
        span: dummy_span,
    }
}

fn parse_into<'src, O>(
    symbols: &mut Interner,
    parser: impl parser::RooParser<'src, O>,
    source: &'src str,
) -> O {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let mut state = parser::State::from(std::mem::take(symbols));
    let result = parser
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");
    *symbols = state.0;
    result
}

fn expr(symbols: &mut Interner, source: &str) -> Expr {
    parse_into(symbols, parser::expr(), source)
}

fn ty(symbols: &mut Interner, source: &str) -> Ty {
    parse_into(symbols, parser::ty(), source)
}

fn pat(symbols: &mut Interner, source: &str) -> Pat {
    parse_into(symbols, parser::pat(parser::expr()), source)
}

fn block(symbols: &mut Interner, source: &str) -> Block {
    parse_into(symbols, parser::block(parser::expr()), source)
}

fn local(symbols: &mut Interner, source: &str) -> Local {
    let mut blk = block(symbols, &format!("{{ {source} }}"));
    let stmt = blk.stmts.remove(0);
    let StmtKind::Let(local) = stmt.kind else {
        panic!("expected a let statement, got {:?}", stmt.kind);
    };
    *local
}

fn resolved_kind(cx: &mut TypeCheckContext<'_>, ty: TyId) -> Option<TyKind> {
    let resolved = cx.inf.resolve(ty);
    cx.inf.ty(resolved).cloned()
}

fn declared_binding(
    cx: &TypeCheckContext<'_>,
    scope: ScopeId,
    namespace: Namespace,
    symbol: &str,
) -> Option<BindingId> {
    let symbol = cx.symbols.get(symbol)?;
    let map = match namespace {
        Namespace::Type => &cx.scopes[scope].types,
        Namespace::Value => &cx.scopes[scope].values,
    };
    map.get(&symbol).copied()
}

#[test]
fn declares_a_free_fn_in_the_value_namespace() {
    let cx = resolve("fn bar() {}");
    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "bar"));
}

#[test]
fn declares_a_symbold_struct_only_in_the_type_namespace() {
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
fn a_struct_and_a_fn_can_share_a_symbol() {
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
fn resolve_path_finds_a_single_segment_symbol() {
    let mut cx = resolve("struct Foo { x: int }");
    let target = path(&mut cx.symbols, &["Foo"]);
    assert!(cx.resolve_path_to_type(&target).is_some());
}

#[test]
fn resolve_path_fails_on_an_undeclared_symbol() {
    let mut cx = resolve("struct Foo { x: int }");
    let target = path(&mut cx.symbols, &["Bar"]);
    assert!(cx.resolve_path_to_type(&target).is_none());
}

#[test]
fn resolve_path_checks_the_requested_namespace() {
    let mut cx = resolve("struct Foo { x: int }");
    let target = path(&mut cx.symbols, &["Foo"]);
    assert!(cx.resolve_path_to_value(&target).is_none());
}

#[test]
fn resolve_path_walks_through_a_module() {
    let mut cx = resolve("mod m { fn baz() {} }");
    let target = path(&mut cx.symbols, &["m", "baz"]);
    let resolved = cx.resolve_path_to_value(&target);
    assert!(resolved.is_some());
}

#[test]
fn resolve_path_rejects_walking_through_a_non_module_segment() {
    let mut cx = resolve("struct Foo { x: int } fn bar() {}");
    let target = path(&mut cx.symbols, &["Foo", "bar"]);
    assert!(cx.resolve_path_to_value(&target).is_none());
}

#[test]
fn resolve_path_module_segment_is_looked_up_by_namespace_not_by_symbol_alone() {
    let mut cx = resolve("mod m { fn baz() {} } fn m() {}");
    let target = path(&mut cx.symbols, &["m", "baz"]);
    assert!(cx.resolve_path_to_value(&target).is_some());
}

#[test]
fn lower_ty_never() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "!");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
}

#[test]
fn lower_ty_paren_unwraps_to_the_inner_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "(!)");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
}

#[test]
fn lower_ty_array_wraps_the_element_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "[!]");
    let t = cx.lower_ty(&ty_val);
    let Some(TyKind::Array(elem)) = resolved_kind(&mut cx, t) else {
        panic!("should be an Array ty");
    };
    assert_eq!(resolved_kind(&mut cx, elem), Some(TyKind::Never));
}

#[test]
fn lower_ty_tup_builds_one_arg_per_element() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "(!, !)");
    let t = cx.lower_ty(&ty_val);
    let Some(TyKind::Tuple(args)) = resolved_kind(&mut cx, t) else {
        panic!("should be a Tuple ty");
    };
    assert_eq!(args.len(), 2);
}

#[test]
fn lower_ty_unit_is_a_zero_arg_tuple() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "()");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
}

#[test]
fn lower_ty_fn_with_explicit_return_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "Fn(!) -> !");
    let t = cx.lower_ty(&ty_val);
    let Some(TyKind::Fn(params, ret)) = resolved_kind(&mut cx, t) else {
        panic!("should be a Fn ty");
    };
    assert_eq!(params.len(), 1);
    assert_eq!(resolved_kind(&mut cx, ret), Some(TyKind::Never));
}

#[test]
fn lower_ty_fn_with_no_return_type_defaults_to_a_fresh_unbound_var() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "Fn(!)");
    let t = cx.lower_ty(&ty_val);
    let Some(TyKind::Fn(_, ret)) = resolved_kind(&mut cx, t) else {
        panic!("should be a Fn ty");
    };
    let resolved = cx.inf.resolve(ret);
    assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
}

#[test]
fn lower_ty_infer_produces_a_fresh_unbound_var() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "_");
    let t = cx.lower_ty(&ty_val);
    let resolved = cx.inf.resolve(t);
    assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
}

#[test]
fn lower_ty_err_is_a_wildcard_that_unifies_with_anything() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let err_ty = Ty {
        kind: AstTyKind::Err,
        span: ast::Span { start: 0, end: 0 },
    };
    let err_ty = cx.lower_ty(&err_ty);
    let int_ty = cx.ty(TyKind::Int);
    assert!(cx.inf.unify(err_ty, int_ty).is_ok());
}

#[test]
fn lower_ty_path_resolves_primitive_symbols() {
    let cases = [
        ("bool", TyKind::Bool),
        ("int", TyKind::Int),
        ("float", TyKind::Float),
        ("char", TyKind::Char),
        ("String", TyKind::Str),
    ];
    for (src, expected) in cases {
        let mut cx = TypeCheckContext::new(Interner::new());
        let ty_val = ty(&mut cx.symbols, src);
        let t = cx.lower_ty(&ty_val);
        assert_eq!(resolved_kind(&mut cx, t), Some(expected), "input: {src}");
    }
}

#[test]
fn lower_ty_path_resolves_a_declared_struct_by_nominal_identity() {
    let mut cx = resolve("struct Foo { x: int }");
    let target = path(&mut cx.symbols, &["Foo"]);
    let binding = cx
        .resolve_path_to_type(&target)
        .expect("Foo should resolve");

    let ty_val = ty(&mut cx.symbols, "Foo");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Struct(binding)));
}

#[test]
fn lower_ty_path_resolves_a_declared_enum_by_nominal_identity() {
    let mut cx = resolve("enum Foo { Bar }");
    let target = path(&mut cx.symbols, &["Foo"]);
    let binding = cx
        .resolve_path_to_type(&target)
        .expect("Foo should resolve");

    let ty_val = ty(&mut cx.symbols, "Foo");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Enum(binding)));
}

#[test]
fn lower_ty_path_to_an_undeclared_symbol_is_err() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let ty_val = ty(&mut cx.symbols, "DoesNotExist");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Err));
}

#[test]
fn lower_ty_path_walks_through_a_module() {
    let mut cx = resolve("mod m { struct Foo; }");
    let target = path(&mut cx.symbols, &["m", "Foo"]);
    let binding = cx
        .resolve_path_to_type(&target)
        .expect("m::Foo should resolve");

    let ty_val = ty(&mut cx.symbols, "m::Foo");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Struct(binding)));
}

#[test]
fn check_expr_bool_literal() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "true");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Bool));
    let expr_val = expr(&mut cx.symbols, "false");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Bool));
}

#[test]
fn check_expr_int_literal() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "5");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn check_expr_float_literal() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "5.0");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Float));
}

#[test]
fn check_expr_str_literal() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "\"hi\"");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Str));
}

#[test]
fn check_expr_char_literal() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "'a'");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Char));
}

#[test]
fn check_expr_paren_has_the_inner_exprs_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "(5)");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn check_expr_err_is_a_wildcard() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let err_expr = Expr {
        annotations: Vec::new(),
        kind: ExprKind::Err,
        span: ast::Span { start: 0, end: 0 },
    };
    let bool_ty = cx.ty(TyKind::Bool);
    let t = cx.check_expr(&err_expr, Some(bool_ty));
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Err));
}

#[test]
fn check_expr_unifies_the_result_against_the_expected_type() {
    let mut cx = resolve("fn foo() {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let never_ty = cx.ty(TyKind::Never);
    let expr_val = expr(&mut cx.symbols, "foo");
    cx.check_expr(&expr_val, Some(never_ty));

    assert_eq!(resolved_kind(&mut cx, binding_ty), Some(TyKind::Never));
}

#[test]
fn check_expr_tup_elements_keep_independent_types() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "(1, \"hi\")");
    let t = cx.check_expr(&expr_val, None);
    let Some(TyKind::Tuple(args)) = resolved_kind(&mut cx, t) else {
        panic!("should be a Tuple ty");
    };
    assert_eq!(resolved_kind(&mut cx, args[0]), Some(TyKind::Int));
    assert_eq!(resolved_kind(&mut cx, args[1]), Some(TyKind::Str));
}

#[test]
fn check_expr_array_elements_are_unified_with_each_other() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "[1, 2, 3]");
    let t = cx.check_expr(&expr_val, None);
    let Some(TyKind::Array(elem)) = resolved_kind(&mut cx, t) else {
        panic!("should be an Array ty");
    };
    assert_eq!(resolved_kind(&mut cx, elem), Some(TyKind::Int));
}

#[test]
fn check_expr_empty_array_uses_the_expected_element_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let never_ty = cx.ty(TyKind::Never);
    let array_of_never = cx.ty(TyKind::Array(never_ty));

    let expr_val = expr(&mut cx.symbols, "[]");
    let t = cx.check_expr(&expr_val, Some(array_of_never));
    let Some(TyKind::Array(elem)) = resolved_kind(&mut cx, t) else {
        panic!("should be an Array ty");
    };
    assert_eq!(resolved_kind(&mut cx, elem), Some(TyKind::Never));
}

#[test]
fn check_expr_path_resolves_to_the_bindings_type() {
    let mut cx = resolve("fn foo() {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let expr_val = expr(&mut cx.symbols, "foo");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(t, binding_ty);
}

#[test]
fn check_expr_path_to_an_undeclared_symbol_is_err() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "doesNotExist");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Err));
}

#[test]
fn check_expr_cast_lowers_the_target_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "5 as float");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Float));
}

#[test]
fn check_expr_call_pins_the_callees_type_to_a_fn_shape() {
    let mut cx = resolve("fn foo() {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let expr_val = expr(&mut cx.symbols, "foo()");
    cx.check_expr(&expr_val, None);

    assert!(matches!(
        resolved_kind(&mut cx, binding_ty),
        Some(TyKind::Fn(..))
    ));
}

#[test]
fn check_expr_call_checks_arguments_against_the_signature() {
    let mut cx = resolve("fn foo() {}");
    let expr_val = expr(&mut cx.symbols, "foo(5)");
    cx.check_expr(&expr_val, None);

    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let Some(TyKind::Fn(input_args, _)) = resolved_kind(&mut cx, binding_ty) else {
        panic!("should be a Fn ty");
    };
    assert_eq!(resolved_kind(&mut cx, input_args[0]), Some(TyKind::Int));
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
fn check_all_referencing_an_undefined_value_is_an_error() {
    let source = r#"
fn use_it() {
    let x = totally_undefined_symbol;
}
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(
        d.message(),
        "cannot find value `totally_undefined_symbol` in this scope"
    );
    assert_eq!(
        &source[d.span().start..d.span().end],
        "totally_undefined_symbol"
    );
}

#[test]
fn check_all_redeclaring_a_function_in_the_same_scope_is_an_error() {
    let source = "fn foo() -> int { 1 } fn foo() -> bool { true }";
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.message(), "the symbol `foo` is defined multiple times");
    assert_eq!(&source[d.span().start..d.span().end], "foo");
    assert_eq!(d.related().len(), 1);
    let (related_span, related_message) = &d.related()[0];
    assert_eq!(&source[related_span.start..related_span.end], "foo");
    assert_eq!(related_message, "previously defined here");
}

#[test]
fn check_all_redeclaring_a_module_in_the_same_scope_is_an_error() {
    let source = "mod m {} mod m {}";
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "the symbol `m` is defined multiple times"
    );
}

#[test]
fn check_all_duplicate_parameter_symbols_are_an_error() {
    let source = "fn use_it(x: int, x: bool) {}";
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "the symbol `x` is defined multiple times"
    );
}

#[test]
fn check_all_let_bindings_can_shadow_each_other_freely() {
    let source = r#"
fn use_it() {
    let x = 1;
    let x = "now a string";
    let x = true;
}
"#;
    let cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());
}

#[test]
fn check_all_a_let_binding_can_shadow_a_parameter_of_the_same_symbol() {
    let source = r#"
fn use_it(x: int) {
    let x = "shadow the param";
}
"#;
    let cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());
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

    let target = path(&mut cx.symbols, &["apply"]);
    let apply = cx
        .resolve_path_to_value(&target)
        .expect("apply should resolve");
    assert_eq!(
        cx.binding(apply).generics().len(),
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
fn check_all_a_directly_self_referential_ty_alias_is_a_cyclic_type_error() {
    let source = "type Foo = (Foo, int);";
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].level(), Level::Error);
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
    assert_eq!(
        &source[diagnostics[0].span().start..diagnostics[0].span().end],
        "(Foo, int)"
    );
}

#[test]
fn check_all_a_mutually_recursive_ty_alias_pair_is_a_cyclic_type_error() {
    let source = r#"
type A = (B, int);
type B = (A, int);
"#;
    let cx = check_all(source);
    let diagnostics = cx.diagnostics();
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
}

#[test]
fn check_all_a_generic_ty_alias_referencing_only_its_own_params_is_not_cyclic() {
    let source = "type Pair<T, U> = (T, U);";
    let cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());
}

#[test]
fn check_expr_call_result_is_an_unbound_var_when_nothing_constrains_it() {
    let mut cx = resolve("fn foo() {}");
    let expr_val = expr(&mut cx.symbols, "foo()");
    let t = cx.check_expr(&expr_val, None);
    let resolved = cx.inf.resolve(t);
    assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
}

#[test]
fn check_expr_ret_with_no_value_is_never() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "return");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
}

#[test]
fn check_expr_ret_with_a_value_is_still_never_not_the_values_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "return 5");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
}

#[test]
fn never_is_a_wildcard_that_unifies_with_anything() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let never_ty = cx.ty(TyKind::Never);
    let int_ty = cx.ty(TyKind::Int);
    assert!(cx.inf.unify(never_ty, int_ty).is_ok());
}

#[test]
fn if_with_no_else_and_a_unit_then_branch_is_unit_typed() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "if true { }");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
}

#[test]
fn if_branches_are_unified_together() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "if true { 1 } else { 2 }");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn if_prefers_the_else_branchs_type_when_the_then_branch_diverges() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "if true { return } else { 5 }");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "if true { 5 } else { return }");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn if_is_never_when_both_branches_diverge() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "if true { return } else { return }");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
}

#[test]
fn if_prefers_the_then_branchs_type_when_the_else_branch_diverges_via_a_semicolon() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let expr_val = expr(&mut cx.symbols, "if true { 5 } else { return 0; }");
    let t = cx.check_expr(&expr_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn check_block_empty_is_unit() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let block_val = block(&mut cx.symbols, "{}");
    let t = cx.check_block(&block_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
}

#[test]
fn check_block_trailing_expr_with_no_semicolon_is_its_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let block_val = block(&mut cx.symbols, "{ 5 }");
    let t = cx.check_block(&block_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn check_block_trailing_expr_with_a_semicolon_does_not_count() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let block_val = block(&mut cx.symbols, "{ 5; }");
    let t = cx.check_block(&block_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Tuple(Vec::new())));
}

#[test]
fn check_block_a_semicolon_tyinated_return_makes_the_block_never() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let block_val = block(&mut cx.symbols, "{ return 0; }");
    let t = cx.check_block(&block_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Never));
}

#[test]
fn check_block_a_non_trailing_let_declares_a_binding_visible_to_later_statements() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let block_val = block(&mut cx.symbols, "{ let x = 5; x }");
    let t = cx.check_block(&block_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Int));
}

#[test]
fn check_block_a_non_trailing_lets_ascription_propagates_to_a_later_reference() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let block_val = block(&mut cx.symbols, "{ let x: float; let y = x; y }");
    let t = cx.check_block(&block_val, None);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Float));
}

#[test]
fn check_pat_ident_declares_a_local_binding() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let never_ty = cx.ty(TyKind::Never);
    let pat_val = pat(&mut cx.symbols, "x");
    cx.check_pat(&pat_val, never_ty, PatDeclKind::Let);

    assert!(lookup(&cx, cx.current_scope, Namespace::Value, "x"));
}

#[test]
fn check_pat_ident_binds_the_locals_type_to_expected() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let never_ty = cx.ty(TyKind::Never);
    let pat_val = pat(&mut cx.symbols, "x");
    cx.check_pat(&pat_val, never_ty, PatDeclKind::Let);

    let binding = declared_binding(&cx, cx.current_scope, Namespace::Value, "x")
        .expect("x should be declared");
    let binding_ty = cx.binding(binding).ty();
    assert_eq!(resolved_kind(&mut cx, binding_ty), Some(TyKind::Never));
}

#[test]
fn check_pat_wild_matches_anything_and_binds_nothing() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let never_ty = cx.ty(TyKind::Never);
    let pat_val = pat(&mut cx.symbols, "_");
    let t = cx.check_pat(&pat_val, never_ty, PatDeclKind::Let);
    assert_eq!(t, never_ty);
    assert!(cx.bindings.is_empty());
}

#[test]
fn check_pat_tuple_declares_one_local_per_position() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let never_ty = cx.ty(TyKind::Never);
    let int_ty = cx.ty(TyKind::Int);
    let expected = cx.ty(TyKind::Tuple(vec![never_ty, int_ty]));

    let pat_val = pat(&mut cx.symbols, "(a, b)");
    cx.check_pat(&pat_val, expected, PatDeclKind::Let);

    let a = declared_binding(&cx, cx.current_scope, Namespace::Value, "a")
        .expect("a should be declared");
    let b = declared_binding(&cx, cx.current_scope, Namespace::Value, "b")
        .expect("b should be declared");
    let a_ty = cx.binding(a).ty();
    let b_ty = cx.binding(b).ty();
    assert_eq!(resolved_kind(&mut cx, a_ty), Some(TyKind::Never));
    assert_eq!(resolved_kind(&mut cx, b_ty), Some(TyKind::Int));
}

#[test]
fn check_pat_tuple_with_no_matching_expected_shape_uses_fresh_vars_per_position() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let int_ty = cx.ty(TyKind::Int);
    let pat_val = pat(&mut cx.symbols, "(a, b)");
    let t = cx.check_pat(&pat_val, int_ty, PatDeclKind::Let);
    let Some(TyKind::Tuple(args)) = resolved_kind(&mut cx, t) else {
        panic!("should be a Tuple ty");
    };
    assert_eq!(args.len(), 2);
}

#[test]
fn check_local_declares_the_pattern_with_the_initializers_type() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let local_val = local(&mut cx.symbols, "let x = 5;");
    cx.check_local(&local_val);

    let binding = declared_binding(&cx, cx.current_scope, Namespace::Value, "x")
        .expect("x should be declared");
    let binding_ty = cx.binding(binding).ty();
    assert_eq!(resolved_kind(&mut cx, binding_ty), Some(TyKind::Int));
}

#[test]
fn check_local_with_no_initializer_uses_the_ascription() {
    let mut cx = TypeCheckContext::new(Interner::new());
    let local_val = local(&mut cx.symbols, "let x: !;");
    cx.check_local(&local_val);

    let binding = declared_binding(&cx, cx.current_scope, Namespace::Value, "x")
        .expect("x should be declared");
    let binding_ty = cx.binding(binding).ty();
    assert_eq!(resolved_kind(&mut cx, binding_ty), Some(TyKind::Never));
}

#[test]
fn check_local_ascription_constrains_the_initializer() {
    let mut cx = resolve("fn foo() {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");

    let local_val = local(&mut cx.symbols, "let x: ! = foo();");
    cx.check_local(&local_val);

    let binding_ty = cx.binding(binding).ty();
    let Some(TyKind::Fn(_, ret)) = resolved_kind(&mut cx, binding_ty) else {
        panic!("should be a Fn ty");
    };
    assert_eq!(resolved_kind(&mut cx, ret), Some(TyKind::Never));
}

#[test]
fn lower_signatures_fn_with_typed_params_and_return() {
    let mut cx = resolve_and_lower("fn add(a: int, b: int) -> float { a }");
    let target = path(&mut cx.symbols, &["add"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("add should resolve");
    let binding_ty = cx.binding(binding).ty();

    let Some(TyKind::Fn(input_args, ret)) = resolved_kind(&mut cx, binding_ty) else {
        panic!("should be a Fn ty");
    };
    assert_eq!(resolved_kind(&mut cx, input_args[0]), Some(TyKind::Int));
    assert_eq!(resolved_kind(&mut cx, input_args[1]), Some(TyKind::Int));
    assert_eq!(resolved_kind(&mut cx, ret), Some(TyKind::Float));
}

#[test]
fn lower_signatures_fn_with_no_return_type_is_a_fresh_unbound_var() {
    let mut cx = resolve_and_lower("fn foo() {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let Some(TyKind::Fn(_, ret)) = resolved_kind(&mut cx, binding_ty) else {
        panic!("should be a Fn ty");
    };
    let resolved = cx.inf.resolve(ret);
    assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
}

#[test]
fn lower_signatures_fn_with_an_untyped_param_gets_a_fresh_var() {
    let mut cx = resolve_and_lower("fn foo(x) {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let Some(TyKind::Fn(input_args, _)) = resolved_kind(&mut cx, binding_ty) else {
        panic!("should be a Fn ty");
    };
    let resolved = cx.inf.resolve(input_args[0]);
    assert!(matches!(cx.inf.ty(resolved), Some(TyKind::Var(_))));
}

#[test]
fn lower_signatures_ty_alias() {
    let mut cx = resolve_and_lower("type MyInt = int;");
    let target = path(&mut cx.symbols, &["MyInt"]);
    let binding = cx
        .resolve_path_to_type(&target)
        .expect("MyInt should resolve");
    let binding_ty = cx.binding(binding).ty();
    assert_eq!(resolved_kind(&mut cx, binding_ty), Some(TyKind::Int));
}

#[test]
fn lower_signatures_recurses_into_a_fns_own_body() {
    let mut cx = resolve_and_lower("fn outer() { fn inner(x: int) -> bool { true } }");
    let body_scope = cx
        .scopes
        .iter()
        .find_map(|(id, scope)| (scope.parent == Some(cx.current_scope)).then_some(id))
        .expect("outer's body should have a child scope");
    let binding = declared_binding(&cx, body_scope, Namespace::Value, "inner")
        .expect("inner should be declared");
    let binding_ty = cx.binding(binding).ty();
    assert!(matches!(
        resolved_kind(&mut cx, binding_ty),
        Some(TyKind::Fn(..))
    ));
}

#[test]
fn lower_signatures_recurses_into_a_mod() {
    let mut cx = resolve_and_lower("mod m { fn baz(x: bool) {} }");
    let target = path(&mut cx.symbols, &["m"]);
    let m_binding = cx.resolve_path_to_type(&target).expect("m should resolve");
    let BindingKind::Mod(m_scope) = &cx.binding(m_binding).kind else {
        panic!("m should be a Mod binding");
    };
    let m_scope = *m_scope;

    let binding =
        declared_binding(&cx, m_scope, Namespace::Value, "baz").expect("baz should resolve");
    let binding_ty = cx.binding(binding).ty();
    assert!(matches!(
        resolved_kind(&mut cx, binding_ty),
        Some(TyKind::Fn(..))
    ));
}

#[test]
fn lower_use_tree_simple_imports_a_value_into_the_current_scope() {
    let mut cx = resolve_and_lower("mod m { fn baz() {} } use m::baz;");

    let target = path(&mut cx.symbols, &["m", "baz"]);
    let original = cx
        .resolve_path_to_value(&target)
        .expect("m::baz should resolve");
    let imported = declared_binding(&cx, cx.current_scope, Namespace::Value, "baz")
        .expect("baz should have been imported into the current scope");

    assert_eq!(imported, original);
}

#[test]
fn lower_use_tree_glob_imports_every_item_from_a_module() {
    let mut cx = resolve_and_lower("mod m { struct Foo; fn baz() {} } use m::*;");

    let foo_target = path(&mut cx.symbols, &["m", "Foo"]);
    let foo_original = cx
        .resolve_path_to_type(&foo_target)
        .expect("m::Foo should resolve");
    let baz_target = path(&mut cx.symbols, &["m", "baz"]);
    let baz_original = cx
        .resolve_path_to_value(&baz_target)
        .expect("m::baz should resolve");

    let foo_imported = declared_binding(&cx, cx.current_scope, Namespace::Type, "Foo")
        .expect("Foo should have been imported into the current scope");
    let baz_imported = declared_binding(&cx, cx.current_scope, Namespace::Value, "baz")
        .expect("baz should have been imported into the current scope");

    assert_eq!(foo_imported, foo_original);
    assert_eq!(baz_imported, baz_original);
}

#[test]
fn lower_use_tree_nested_imports_each_item_in_the_group_and_honours_resymbols() {
    let mut cx =
        resolve_and_lower("mod m { struct Foo; fn baz() {} } use m::{Foo, baz as make_baz};");

    let foo_target = path(&mut cx.symbols, &["m", "Foo"]);
    let foo_original = cx
        .resolve_path_to_type(&foo_target)
        .expect("m::Foo should resolve");
    let baz_target = path(&mut cx.symbols, &["m", "baz"]);
    let baz_original = cx
        .resolve_path_to_value(&baz_target)
        .expect("m::baz should resolve");

    let foo_imported = declared_binding(&cx, cx.current_scope, Namespace::Type, "Foo")
        .expect("Foo should have been imported into the current scope");
    let make_baz_imported = declared_binding(&cx, cx.current_scope, Namespace::Value, "make_baz")
        .expect("baz should have been imported as make_baz");

    assert_eq!(foo_imported, foo_original);
    assert_eq!(make_baz_imported, baz_original);
    assert!(
        declared_binding(&cx, cx.current_scope, Namespace::Value, "baz").is_none(),
        "baz should not also be imported under its original symbol"
    );
}

#[test]
fn lower_signatures_makes_the_declared_signature_authoritative() {
    let mut cx = resolve_and_lower("fn foo(x: int) {}");
    let target = path(&mut cx.symbols, &["foo"]);
    let binding = cx
        .resolve_path_to_value(&target)
        .expect("foo should resolve");
    let binding_ty = cx.binding(binding).ty();

    let expr_val = expr(&mut cx.symbols, "foo(\"wrong\")");
    cx.check_expr(&expr_val, None);

    let Some(TyKind::Fn(input_args, _)) = resolved_kind(&mut cx, binding_ty) else {
        panic!("should still be a Fn ty");
    };
    assert_eq!(resolved_kind(&mut cx, input_args[0]), Some(TyKind::Int));
}

fn check_all(source: &str) -> TypeCheckContext<'static> {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");
    let items: &'static [Box<Item>] = Vec::leak(items);

    let mut cx = TypeCheckContext::new(state.0);
    cx.resolve(items);
    cx.lower_signatures(items);
    cx.check(items);
    cx
}

fn fn_body_scope(cx: &TypeCheckContext<'_>, binding: BindingId) -> ScopeId {
    match &cx.binding(binding).kind {
        BindingKind::Fn(fn_data) => fn_data.scope,
        _ => panic!("expected a Fn binding"),
    }
}

fn generics_list(generic_names: &GenericNames, generics: &[GenericId]) -> String {
    if generics.is_empty() {
        return String::new();
    }
    let symbols: Vec<String> = generics
        .iter()
        .map(|id| {
            generic_names
                .get(id)
                .cloned()
                .unwrap_or_else(|| "<generic>".to_owned())
        })
        .collect();
    format!("<{}>", symbols.join(", "))
}

struct Renderer<'a> {
    inf: &'a mut InferenceTable,
    bindings: &'a SlotMap<BindingId, Binding>,
    symbols: &'a Interner,
    generic_names: &'a GenericNames,
}

impl<'ast> TypeCheckContext<'ast> {
    fn renderer(&mut self) -> Renderer<'_> {
        Renderer {
            inf: &mut self.inf,
            bindings: &self.bindings,
            symbols: &self.symbols,
            generic_names: &self.generic_names,
        }
    }

    fn render_binding_type(&mut self, binding: BindingId) -> String {
        self.renderer().render_binding_type(binding)
    }

    fn describe_binding(&mut self, binding: BindingId) -> String {
        self.renderer().describe_binding(binding)
    }
}

impl Renderer<'_> {
    fn render_ty(&mut self, ty: TyId) -> String {
        let mut buf = String::new();
        self.render_ty_into(&mut buf, ty, None);
        buf
    }

    fn render_ty_into(
        &mut self,
        buf: &mut String,
        ty: TyId,
        highlight: Option<TyId>,
    ) -> Option<Range<usize>> {
        if let Some(highlight) = highlight {
            if self.inf.resolve(ty) == self.inf.resolve(highlight) {
                let start = buf.len();
                buf.push_str(&self.render_ty(ty));
                return Some(start..buf.len());
            }
        }

        let resolved = self.inf.resolve(ty);
        let Some(kind) = self.inf.ty(resolved).cloned() else {
            buf.push_str("<error>");
            return None;
        };

        match kind {
            TyKind::Var(_) => {
                buf.push('_');
                None
            }
            TyKind::Any => {
                buf.push_str("any");
                None
            }
            TyKind::Never => {
                buf.push('!');
                None
            }
            TyKind::Int => {
                buf.push_str("int");
                None
            }
            TyKind::Float => {
                buf.push_str("float");
                None
            }
            TyKind::Bool => {
                buf.push_str("bool");
                None
            }
            TyKind::Char => {
                buf.push_str("char");
                None
            }
            TyKind::Str => {
                buf.push_str("String");
                None
            }
            TyKind::Err => {
                buf.push_str("<error>");
                None
            }
            TyKind::Array(elem) => {
                buf.push('[');
                let range = self.render_ty_into(buf, elem, highlight);
                buf.push(']');
                range
            }
            TyKind::Tuple(args) => {
                buf.push('(');
                let mut range = None;
                for (i, arg) in args.into_iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    range = range.or(self.render_ty_into(buf, arg, highlight));
                }
                buf.push(')');
                range
            }
            TyKind::Fn(params, output) => {
                buf.push_str("Fn(");
                let mut inputs_range = None;
                for (i, arg) in params.into_iter().enumerate() {
                    if i > 0 {
                        buf.push_str(", ");
                    }
                    inputs_range = inputs_range.or(self.render_ty_into(buf, arg, highlight));
                }
                buf.push(')');
                buf.push_str(" -> ");
                let output_range = self.render_ty_into(buf, output, highlight);
                inputs_range.or(output_range)
            }
            TyKind::Struct(binding) | TyKind::Enum(binding) => {
                let symbol = self.bindings[binding].symbol;
                buf.push_str(self.symbols.resolve(symbol));
                None
            }
            TyKind::Generic(id) => {
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

    fn render_binding_type(&mut self, binding: BindingId) -> String {
        let ty = self.bindings[binding].ty();
        let rendered = self.render_ty(ty);
        let generics = self.bindings[binding].generics();
        let generics_rendered = generics_list(self.generic_names, generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    fn describe_binding(&mut self, binding: BindingId) -> String {
        match &self.bindings[binding].kind {
            BindingKind::Fn(_) => self.describe_fn_item(binding),
            BindingKind::Param(_) => {
                let symbol = self.binding_display_symbol(binding);
                let ty = self.render_binding_type(binding);
                format!("{symbol}: {ty}")
            }
            BindingKind::Local(_) => {
                let symbol = self.binding_display_symbol(binding);
                let ty = self.render_binding_type(binding);
                format!("let {symbol}: {ty}")
            }
            BindingKind::TyAlias(_) => {
                format!("type {}", self.alias_symbol_with_generics(binding))
            }
            BindingKind::Mod(_) => format!("mod {}", self.binding_display_symbol(binding)),
            BindingKind::Struct
            | BindingKind::Enum
            | BindingKind::Variant
            | BindingKind::Trait
            | BindingKind::GenericParam(_) => self.render_binding_type(binding),
        }
    }

    fn binding_display_symbol(&mut self, binding: BindingId) -> String {
        let symbol = self.bindings[binding].symbol;
        self.symbols.resolve(symbol).to_owned()
    }

    fn alias_symbol_with_generics(&mut self, binding: BindingId) -> String {
        let symbol = self.binding_display_symbol(binding);
        let generics = self.bindings[binding].generics();
        let generics_rendered = generics_list(self.generic_names, generics);
        format!("{symbol}{generics_rendered}")
    }

    fn describe_fn_item(&mut self, binding: BindingId) -> String {
        let symbol = self.bindings[binding].symbol;
        let symbol = self.symbols.resolve(symbol).to_owned();

        let generics = self.bindings[binding].generics();
        let generics_rendered = generics_list(self.generic_names, generics);

        let BindingKind::Fn(FnBinding { param_symbols, .. }) = &self.bindings[binding].kind else {
            unreachable!("describe_fn_item is only ever called for a BindingKind::Fn binding");
        };
        let param_symbols = param_symbols.clone();

        let ty = self.bindings[binding].ty();
        let resolved = self.inf.resolve(ty);
        let Some(TyKind::Fn(param_types, output)) = self.inf.ty(resolved).cloned() else {
            return self.render_binding_type(binding);
        };

        let params: Vec<String> = param_types
            .iter()
            .enumerate()
            .map(|(i, &ty)| {
                let rendered = self.render_ty(ty);
                match param_symbols.get(i) {
                    Some(symbol) => format!("{symbol}: {rendered}"),
                    None => rendered,
                }
            })
            .collect();

        let output_rendered = self.render_ty(output);
        format!(
            "fn {symbol}{generics_rendered}({}) -> {output_rendered}",
            params.join(", ")
        )
    }
}

#[test]
fn check_all_infers_an_untyped_params_type_from_the_bodys_declared_return_type() {
    let mut cx = check_all("fn identity(x) -> int { x }");
    let target = path(&mut cx.symbols, &["identity"]);
    let fn_binding = cx
        .resolve_path_to_value(&target)
        .expect("identity should resolve");
    let body_scope = fn_body_scope(&cx, fn_binding);

    let x_binding = declared_binding(&cx, body_scope, Namespace::Value, "x")
        .expect("x should be declared as a param");
    let x_ty = cx.binding(x_binding).ty();
    assert_eq!(resolved_kind(&mut cx, x_ty), Some(TyKind::Int));
}

#[test]
fn check_all_recurses_into_a_nested_fns_body() {
    let mut cx = check_all("fn outer() { fn inner(x) -> int { x } }");
    let target = path(&mut cx.symbols, &["outer"]);
    let outer_binding = cx
        .resolve_path_to_value(&target)
        .expect("outer should resolve");
    let outer_scope = fn_body_scope(&cx, outer_binding);

    let inner_binding = declared_binding(&cx, outer_scope, Namespace::Value, "inner")
        .expect("inner should be declared inside outer's body");
    let inner_scope = fn_body_scope(&cx, inner_binding);

    let x_binding = declared_binding(&cx, inner_scope, Namespace::Value, "x")
        .expect("x should be declared as inner's param");
    let x_ty = cx.binding(x_binding).ty();
    assert_eq!(resolved_kind(&mut cx, x_ty), Some(TyKind::Int));
}

#[test]
fn check_all_nested_fn_body_resolves_a_reference_to_an_outer_params_binding() {
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

    let decl_binding = cx
        .binding_at(param_decl_offset)
        .expect("should resolve at outer's parameter declaration");
    let use_binding = cx
        .binding_at(param_use_offset)
        .expect("inner's reference to x should resolve to outer's parameter");
    assert_eq!(decl_binding, use_binding);
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
fn binding_at_finds_a_fns_own_declaration_and_every_later_reference_to_it() {
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

    let decl_binding = cx
        .binding_at(decl_offset)
        .expect("should resolve at the declaration");
    let use_binding = cx
        .binding_at(use_offset)
        .expect("should resolve at the call site");
    assert_eq!(decl_binding, use_binding);
    assert_eq!(cx.render_binding_type(decl_binding), "Fn(int) -> int");
}

#[test]
fn binding_at_finds_a_parameter_at_both_its_declaration_and_its_use_in_the_body() {
    let source = "fn add_one(x: int) -> int { x }";
    let mut cx = check_all(source);

    let param_decl = source.find("x:").unwrap();
    let param_use = source.rfind('x').unwrap();
    assert_ne!(param_decl, param_use);

    let decl_binding = cx
        .binding_at(param_decl)
        .expect("should resolve at the parameter");
    let use_binding = cx
        .binding_at(param_use)
        .expect("should resolve at the body reference");
    assert_eq!(decl_binding, use_binding);
    assert_eq!(cx.render_binding_type(decl_binding), "int");
}

#[test]
fn binding_at_finds_a_let_bindings_inferred_type() {
    let source = r#"
fn use_it() {
    let n = 1;
}
"#;
    let mut cx = check_all(source);
    let offset = source.find("let n").unwrap() + "let ".len();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the let binding");
    assert_eq!(cx.render_binding_type(binding), "int");
}

#[test]
fn binding_at_finds_a_struct_symbol_at_its_declaration_and_in_a_type_annotation() {
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

    let decl_binding = cx
        .binding_at(decl_offset)
        .expect("should resolve at the struct decl");
    let use_binding = cx
        .binding_at(use_offset)
        .expect("should resolve at the annotation");
    assert_eq!(decl_binding, use_binding);
}

#[test]
fn binding_at_finds_a_generic_param_at_its_declaration_and_in_the_signature() {
    let source = "fn identity<T>(x: T) -> T { x }";
    let cx = check_all(source);

    let decl_offset = source.find('T').unwrap();
    let param_ty_offset = source.find("x: T").unwrap() + 3;
    let ret_ty_offset = source.rfind('T').unwrap();

    let decl_binding = cx.binding_at(decl_offset).expect("should resolve at <T>");
    let param_binding = cx
        .binding_at(param_ty_offset)
        .expect("should resolve at x: T");
    let ret_binding = cx
        .binding_at(ret_ty_offset)
        .expect("should resolve at -> T");
    assert_eq!(decl_binding, param_binding);
    assert_eq!(decl_binding, ret_binding);
}

#[test]
fn binding_at_is_none_between_identifiers() {
    let source = "fn add_one(x: int) -> int { x }";
    let cx = check_all(source);
    let space_offset = source.find(") ->").unwrap();
    assert_eq!(cx.binding_at(space_offset), None);
}

#[test]
fn describe_binding_a_fn_item_uses_source_declaration_syntax_with_param_symbols() {
    let source = "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U { f(g(x)) }";
    let mut cx = check_all(source);
    let offset = source.find("compose").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the fn's own symbol");
    assert_eq!(
        cx.describe_binding(binding),
        "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U"
    );
}

#[test]
fn describe_binding_a_parameter_is_prefixed_with_its_own_symbol() {
    let source = "fn add_one(x: int) -> int { x }";
    let mut cx = check_all(source);
    let offset = source.find("x:").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(cx.describe_binding(binding), "x: int");
}

#[test]
fn describe_binding_a_let_binding_is_prefixed_with_let_and_its_own_symbol() {
    let source = r#"
fn use_it() {
    let n = 1;
}
"#;
    let mut cx = check_all(source);
    let offset = source.find("let n").unwrap() + "let ".len();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the let binding");
    assert_eq!(cx.describe_binding(binding), "let n: int");
}

#[test]
fn describe_binding_a_higher_order_parameter_keeps_the_bare_fn_type_syntax() {
    let source = "fn apply<T, U>(f: Fn(T) -> U, x: T) -> U { f(x) }";
    let mut cx = check_all(source);
    let offset = source.find("f:").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(cx.describe_binding(binding), "f: Fn(T) -> U");
}

#[test]
fn describe_binding_a_ty_alias_declaration_shows_the_type_keyword() {
    let source = "type Pair<T, U> = (T, U);";
    let mut cx = check_all(source);
    let offset = source.find("Pair").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the alias's own symbol");
    assert_eq!(cx.describe_binding(binding), "type Pair<T, U>");
}

#[test]
fn describe_binding_a_ty_alias_reference_also_shows_the_type_keyword() {
    let source = r#"
type Pair<T, U> = (T, U);
fn make_pair<T, U>(a: T, b: U) -> Pair<T, U> {
    (a, b)
}
"#;
    let mut cx = check_all(source);
    let offset = source.rfind("Pair").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the return-type reference");
    assert_eq!(cx.describe_binding(binding), "type Pair<T, U>");
}

#[test]
fn describe_binding_a_mod_declaration_shows_the_mod_keyword() {
    let source = "mod example { fn foo() {} }";
    let mut cx = check_all(source);
    let offset = source.find("example").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the module's own symbol");
    assert_eq!(cx.describe_binding(binding), "mod example");
}

#[test]
fn describe_binding_a_mod_reference_also_shows_the_mod_keyword() {
    let source = r#"
mod outer {
    mod inner {
        fn foo() {}
    }
}
use outer::inner;
"#;
    let mut cx = check_all(source);
    let offset = source.rfind("inner").unwrap();
    let binding = cx
        .binding_at(offset)
        .expect("should resolve at the use path's reference to the module");
    assert_eq!(cx.describe_binding(binding), "mod inner");
}

#[test]
fn type_symbol_at_finds_every_literal_kinds_own_type() {
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
    assert_eq!(cx.type_symbol_at(source.find('1').unwrap()), Some("int"));
    assert_eq!(
        cx.type_symbol_at(source.find("1.5").unwrap()),
        Some("float")
    );
    assert_eq!(cx.type_symbol_at(source.find("'x'").unwrap()), Some("char"));
    assert_eq!(
        cx.type_symbol_at(source.find("true").unwrap()),
        Some("bool")
    );
    assert_eq!(
        cx.type_symbol_at(source.find("\"hi\"").unwrap()),
        Some("String")
    );
}

#[test]
fn type_symbol_at_finds_a_primitive_symbol_in_an_ordinary_type_annotation() {
    let source = "fn add_one(x: int) -> int { x }";
    let cx = check_all(source);
    let offset = source.find("int").unwrap();
    assert_eq!(cx.type_symbol_at(offset), Some("int"));
}

#[test]
fn type_symbol_at_finds_a_primitive_generic_argument_in_a_turbofish() {
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
    assert_eq!(cx.type_symbol_at(offset), Some("int"));
}

#[test]
fn type_symbol_at_is_none_away_from_any_literal_or_primitive_symbol() {
    let source = "fn add_one(x: int) -> int { x }";
    let cx = check_all(source);
    let offset = source.find("add_one").unwrap();
    assert_eq!(cx.type_symbol_at(offset), None);
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

    let ping_target = path(&mut cx.symbols, &["ping"]);
    let ping = cx
        .resolve_path_to_value(&ping_target)
        .expect("ping should resolve");
    let pong_target = path(&mut cx.symbols, &["pong"]);
    let pong = cx
        .resolve_path_to_value(&pong_target)
        .expect("pong should resolve");
    assert_eq!(cx.binding(ping).generics().len(), 1);
    assert_eq!(cx.binding(pong).generics().len(), 1);
    assert_eq!(
        cx.binding(ping).generics()[0],
        cx.binding(pong).generics()[0]
    );
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

    let helper_target = path(&mut cx.symbols, &["helper"]);
    let helper = cx
        .resolve_path_to_value(&helper_target)
        .expect("helper should resolve");
    let caller_target = path(&mut cx.symbols, &["caller"]);
    let caller = cx
        .resolve_path_to_value(&caller_target)
        .expect("caller should resolve");
    assert_eq!(cx.binding(helper).generics().len(), 1);
    assert_eq!(cx.binding(caller).generics().len(), 1);
}

#[test]
fn check_all_a_parameter_shadowing_a_siblings_symbol_is_not_treated_as_a_call_to_it() {
    let source = r#"
fn apply(f, x) {
    f(x)
}
fn f(x) {
    apply(x, x)
}
"#;
    let mut cx = check_all(source);

    let target = path(&mut cx.symbols, &["apply"]);
    let apply = cx
        .resolve_path_to_value(&target)
        .expect("apply should resolve");
    assert_eq!(
        cx.render_binding_type(apply),
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
fn check_all_a_let_binding_shadowing_a_siblings_symbol_is_not_treated_as_a_call_to_it() {
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

    let target = path(&mut cx.symbols, &["apply"]);
    let apply = cx
        .resolve_path_to_value(&target)
        .expect("apply should resolve");
    assert_eq!(
        cx.render_binding_type(apply),
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

    let target = path(&mut cx.symbols, &["identity_rec"]);
    let identity_rec = cx
        .resolve_path_to_value(&target)
        .expect("identity_rec should resolve");
    assert_eq!(cx.binding(identity_rec).generics().len(), 1);
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

    for symbol in ["a", "b", "c"] {
        let target = path(&mut cx.symbols, &[symbol]);
        let binding = cx
            .resolve_path_to_value(&target)
            .unwrap_or_else(|| panic!("{symbol} should resolve"));
        assert_eq!(cx.binding(binding).generics().len(), 1, "{symbol}");
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

    let ping2_target = path(&mut cx.symbols, &["ping2"]);
    let ping2 = cx
        .resolve_path_to_value(&ping2_target)
        .expect("ping2 should resolve");
    let pong2_target = path(&mut cx.symbols, &["pong2"]);
    let pong2 = cx
        .resolve_path_to_value(&pong2_target)
        .expect("pong2 should resolve");
    assert_eq!(cx.binding(ping2).generics().len(), 0);
    assert_eq!(cx.binding(pong2).generics().len(), 0);
}

#[test]
fn check_all_a_newly_generalized_param_never_reuses_an_explicit_generics_symbol() {
    let source = r#"
fn compose<T>(f, g: Fn(int) -> _, x) -> Fn(T) -> String {
    f(g(x))
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let target = path(&mut cx.symbols, &["compose"]);
    let compose = cx
        .resolve_path_to_value(&target)
        .expect("compose should resolve");
    let generics = cx.binding(compose).generics();
    assert_eq!(generics.len(), 2, "{generics:?}");

    let explicit = generics[0];
    let inferred = generics[1];
    assert_ne!(explicit, inferred, "should be two distinct GenericIds");

    let explicit_symbol = cx.generic_names.get(&explicit).cloned();
    let inferred_symbol = cx.generic_names.get(&inferred).cloned();
    assert_eq!(explicit_symbol.as_deref(), Some("T"));
    assert_ne!(
        explicit_symbol, inferred_symbol,
        "the newly-generalized parameter must not render under the \
             same symbol as the explicit `<T>`"
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

    let target = path(&mut cx.symbols, &["outer"]);
    let outer = cx
        .resolve_path_to_value(&target)
        .expect("outer should resolve");
    let outer_scope = fn_body_scope(&cx, outer);

    let ping = declared_binding(&cx, outer_scope, Namespace::Value, "ping")
        .expect("ping should be declared inside outer's body");
    let pong = declared_binding(&cx, outer_scope, Namespace::Value, "pong")
        .expect("pong should be declared inside outer's body");
    assert_eq!(cx.binding(ping).generics().len(), 1);
    assert_eq!(cx.binding(pong).generics().len(), 1);
}

#[test]
fn check_all_mutual_recursion_reached_only_through_a_nested_fn_value_is_one_scc() {
    let source = r#"
fn outer_1(x) {
    fn inner_1(y) {
        outer_2(y)
    }
    let _inner_1 = inner_1;
    _inner_1(x)
}

fn outer_2(x) {
    fn inner_2(y) {
        outer_1(y)
    }
    let _inner_2 = inner_2;
    _inner_2(x)
}
"#;
    let mut cx = check_all(source);
    assert!(cx.diagnostics().is_empty(), "{:#?}", cx.diagnostics());

    let outer_1_target = path(&mut cx.symbols, &["outer_1"]);
    let outer_1 = cx
        .resolve_path_to_value(&outer_1_target)
        .expect("outer_1 should resolve");
    let outer_2_target = path(&mut cx.symbols, &["outer_2"]);
    let outer_2 = cx
        .resolve_path_to_value(&outer_2_target)
        .expect("outer_2 should resolve");
    let inner_1 = declared_binding(
        &cx,
        fn_body_scope(&cx, outer_1),
        Namespace::Value,
        "inner_1",
    )
    .expect("inner_1 should be declared inside outer_1's body");
    let inner_2 = declared_binding(
        &cx,
        fn_body_scope(&cx, outer_2),
        Namespace::Value,
        "inner_2",
    )
    .expect("inner_2 should be declared inside outer_2's body");

    let expected = "<T, U> Fn(T) -> U";
    assert_eq!(cx.render_binding_type(outer_1), expected);
    assert_eq!(cx.render_binding_type(outer_2), expected);
    assert_eq!(cx.render_binding_type(inner_1), expected);
    assert_eq!(cx.render_binding_type(inner_2), expected);
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

    let target = path(&mut cx.symbols, &["compose"]);
    let compose = cx
        .resolve_path_to_value(&target)
        .expect("compose should resolve");
    let compose_scope = fn_body_scope(&cx, compose);

    let inner = declared_binding(&cx, compose_scope, Namespace::Value, "inner")
        .expect("inner should be declared inside compose's body");
    let inner_scope = fn_body_scope(&cx, inner);
    let innermost = declared_binding(&cx, inner_scope, Namespace::Value, "innermost")
        .expect("innermost should be declared inside inner's body");

    assert_eq!(
        cx.binding(compose).generics().len(),
        3,
        "{:#?}",
        cx.binding(compose).generics()
    );
    // `inner` legitimately generalizes 1 variable of its own: the type shared
    // between `innermost`'s parameter `x` and `inner`'s own parameter `g`'s
    // domain. That variable is not free in the enclosing signature (`f`'s
    // domain/codomain never mention it), so under standard let-polymorphism
    // it's sound for `inner` to generalize it rather than deferring to
    // `compose`. The other 2 variables that stay free at this point (`f`'s
    // domain and codomain) are correctly excluded, and end up on `compose`
    // instead -- which is what this test is actually asserting.
    assert_eq!(cx.binding(inner).generics().len(), 1);
    assert_eq!(cx.binding(innermost).generics().len(), 0);
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
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");
    CheckedProgram::check(&items, state.0)
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
fn freeze_render_binding_type_a_generic_fn() {
    let source = "fn identity<T>(x: T) -> T { x }";
    let frozen = check_all_frozen(source);
    let offset = source.find("identity").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the fn's own symbol");
    assert_eq!(frozen.render_binding_type(binding), "<T> Fn(T) -> T");
}

#[test]
fn freeze_render_binding_type_a_struct_parameter() {
    let source = r#"
struct Point {
    x: int,
}
fn use_it(p: Point) {}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.find("p: Point").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(frozen.render_binding_type(binding), "Point");
}

#[test]
fn freeze_render_binding_type_a_mutually_recursive_pair_generalized_together() {
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
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at apply's own symbol");
    assert_eq!(
        frozen.render_binding_type(binding),
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );
}

#[test]
fn freeze_describe_binding_a_fn_item_uses_source_declaration_syntax_with_param_symbols() {
    let source = "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U { f(g(x)) }";
    let frozen = check_all_frozen(source);
    let offset = source.find("compose").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the fn's own symbol");
    assert_eq!(
        frozen.describe_binding(binding),
        "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U"
    );
}

#[test]
fn freeze_describe_binding_a_parameter_is_prefixed_with_its_own_symbol() {
    let source = "fn add_one(x: int) -> int { x }";
    let frozen = check_all_frozen(source);
    let offset = source.find("x:").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(frozen.describe_binding(binding), "x: int");
}

#[test]
fn freeze_describe_binding_a_let_binding_is_prefixed_with_let_and_its_own_symbol() {
    let source = r#"
fn use_it() {
    let n = 1;
}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.find("let n").unwrap() + "let ".len();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the let binding");
    assert_eq!(frozen.describe_binding(binding), "let n: int");
}

#[test]
fn freeze_describe_binding_a_higher_order_parameter_keeps_the_bare_fn_type_syntax() {
    let source = "fn apply<T, U>(f: Fn(T) -> U, x: T) -> U { f(x) }";
    let frozen = check_all_frozen(source);
    let offset = source.find("f:").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the parameter");
    assert_eq!(frozen.describe_binding(binding), "f: Fn(T) -> U");
}

#[test]
fn freeze_describe_binding_a_ty_alias_declaration_shows_the_type_keyword() {
    let source = "type Pair<T, U> = (T, U);";
    let frozen = check_all_frozen(source);
    let offset = source.find("Pair").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the alias's own symbol");
    assert_eq!(frozen.describe_binding(binding), "type Pair<T, U>");
}

#[test]
fn freeze_describe_binding_a_ty_alias_reference_also_shows_the_type_keyword() {
    let source = r#"
type Pair<T, U> = (T, U);
fn make_pair<T, U>(a: T, b: U) -> Pair<T, U> {
    (a, b)
}
"#;
    let frozen = check_all_frozen(source);
    let offset = source.rfind("Pair").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the return-type reference");
    assert_eq!(frozen.describe_binding(binding), "type Pair<T, U>");
}

#[test]
fn freeze_describe_binding_a_mod_declaration_shows_the_mod_keyword() {
    let source = "mod example { fn foo() {} }";
    let frozen = check_all_frozen(source);
    let offset = source.find("example").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the module's own symbol");
    assert_eq!(frozen.describe_binding(binding), "mod example");
}

#[test]
fn freeze_describe_binding_a_mod_reference_also_shows_the_mod_keyword() {
    let source = r#"
mod outer {
    mod inner {
        fn foo() {}
    }
}
use outer::inner;
"#;
    let frozen = check_all_frozen(source);
    let offset = source.rfind("inner").unwrap();
    let binding = frozen
        .binding_at(offset)
        .expect("should resolve at the use path's reference to the module");
    assert_eq!(frozen.describe_binding(binding), "mod inner");
}

#[test]
fn freeze_type_symbol_at_still_finds_a_literals_type() {
    let source = "fn use_it() { 1; }";
    let frozen = check_all_frozen(source);
    let offset = source.find('1').unwrap();
    assert_eq!(frozen.type_symbol_at(offset), Some("int"));
}
