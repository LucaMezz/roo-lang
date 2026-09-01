pub use super::*;
use chumsky::Parser;
pub use indoc::indoc;
use std::ops::Range;

use ast::{Block, Expr, Local, Pat, StmtKind};
pub use intern::Interner;

impl<'ast> TypeCheckContext<'ast> {
    pub(crate) fn def_at(&self, offset: usize) -> Option<DefId> {
        self.positions.def_at(offset)
    }

    pub(crate) fn type_symbol_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }
}

pub(crate) fn resolve<'ast>(source: &str) -> TypeCheckContext<'ast> {
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

pub(crate) fn resolve_and_lower<'ast>(source: &str) -> TypeCheckContext<'ast> {
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

pub(crate) fn check_all(source: &str) -> TypeCheckContext<'static> {
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

pub(crate) fn lookup(
    cx: &TypeCheckContext<'_>,
    scope: ScopeId,
    namespace: Namespace,
    symbol: &str,
) -> bool {
    let Some(symbol) = cx.symbols.get(symbol) else {
        return false;
    };
    cx.scopes.lookup(scope, symbol, namespace).is_some()
}

pub(crate) fn path(symbols: &mut Interner, segments: &[&str]) -> Path {
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

pub(crate) fn parse_into<'src, O>(
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

pub(crate) fn expr(symbols: &mut Interner, source: &str) -> Expr {
    parse_into(symbols, parser::expr(), source)
}

pub(crate) fn ty(symbols: &mut Interner, source: &str) -> Ty {
    parse_into(symbols, parser::ty(), source)
}

pub(crate) fn pat(symbols: &mut Interner, source: &str) -> Pat {
    parse_into(symbols, parser::pat(parser::expr()), source)
}

pub(crate) fn block(symbols: &mut Interner, source: &str) -> Block {
    parse_into(symbols, parser::block(parser::expr()), source)
}

pub(crate) fn local(symbols: &mut Interner, source: &str) -> Local {
    let mut blk = block(symbols, &format!("{{ {source} }}"));
    let stmt = blk.stmts.remove(0);
    let StmtKind::Let(local) = stmt.kind else {
        panic!("expected a let statement, got {:?}", stmt.kind);
    };
    *local
}

pub(crate) fn fn_body_scope(cx: &TypeCheckContext<'_>, def: DefId) -> ScopeId {
    match &cx.def(def).kind {
        DefKind::Fn(fn_data) => fn_data.scope,
        _ => panic!("expected a Fn def"),
    }
}

pub(crate) struct Renderer<'a, 'ast> {
    cx: &'a mut TypeCheckContext<'ast>,
}

impl<'ast> TypeCheckContext<'ast> {
    pub(crate) fn renderer(&mut self) -> Renderer<'_, 'ast> {
        Renderer { cx: self }
    }

    // fn render_def_type(&mut self, def: DefId) -> String {
    //     self.renderer().render_def_type(def)
    // }
    //
    // fn describe_def(&mut self, def: DefId) -> String {
    //     self.renderer().describe_def(def)
    // }
}

impl Renderer<'_, '_> {
    pub(crate) fn render_generic_param(&mut self, id: DefIdOf<GenericParamDef>) -> String {
        let name = self.cx.generic_name(id);
        let bounds = self.cx.defs.generic_param_ref(id).bounds.clone();
        if bounds.is_empty() {
            return name;
        }
        let rendered: Vec<String> = bounds.iter().map(|&bound| self.render_ty(bound)).collect();
        format!("{name}: {}", rendered.join(" + "))
    }

    pub(crate) fn generics_list(&mut self, generics: &[DefIdOf<GenericParamDef>]) -> String {
        if generics.is_empty() {
            return String::new();
        }
        let params: Vec<String> = generics
            .to_vec()
            .into_iter()
            .map(|id| self.render_generic_param(id))
            .collect();
        format!("<{}>", params.join(", "))
    }

    pub(crate) fn render_ty(&mut self, ty: TyId) -> String {
        let mut buf = String::new();
        self.render_ty_into(&mut buf, ty, None);
        buf
    }

    pub(crate) fn render_ty_into(
        &mut self,
        buf: &mut String,
        ty: TyId,
        highlight: Option<TyId>,
    ) -> Option<Range<usize>> {
        if let Some(highlight) = highlight {
            if self.cx.inf.resolve(ty) == self.cx.inf.resolve(highlight) {
                let start = buf.len();
                buf.push_str(&self.render_ty(ty));
                return Some(start..buf.len());
            }
        }

        let resolved = self.cx.inf.resolve(ty);
        let Some(kind) = self.cx.inf.ty(resolved).cloned() else {
            buf.push_str("<error>");
            return None;
        };

        match kind {
            TyKind::Var(_) => {
                buf.push('_');
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
            TyKind::Struct(def, args) => {
                let symbol = self.cx.defs.get(def.id()).symbol;
                buf.push_str(self.cx.symbols.resolve(symbol));
                if !args.is_empty() {
                    buf.push('<');
                    for (i, arg) in args.into_iter().enumerate() {
                        if i > 0 {
                            buf.push_str(", ");
                        }
                        self.render_ty_into(buf, arg, highlight);
                    }
                    buf.push('>');
                }
                None
            }
            TyKind::Enum(def, args) => {
                let symbol = self.cx.defs.get(def.id()).symbol;
                buf.push_str(self.cx.symbols.resolve(symbol));
                if !args.is_empty() {
                    buf.push('<');
                    for (i, arg) in args.into_iter().enumerate() {
                        if i > 0 {
                            buf.push_str(", ");
                        }
                        self.render_ty_into(buf, arg, highlight);
                    }
                    buf.push('>');
                }
                None
            }
            TyKind::TraitObject(def, args) => {
                let symbol = self.cx.defs.get(def.id()).symbol;
                buf.push_str(self.cx.symbols.resolve(symbol));
                if !args.is_empty() {
                    buf.push('<');
                    for (i, arg) in args.into_iter().enumerate() {
                        if i > 0 {
                            buf.push_str(", ");
                        }
                        self.render_ty_into(buf, arg, highlight);
                    }
                    buf.push('>');
                }
                None
            }
            TyKind::Generic(id) => {
                buf.push_str(&self.cx.generic_name(id));
                None
            }
        }
    }

    pub(crate) fn render_def_type(&mut self, def: DefId) -> String {
        let ty = self.cx.defs.get(def).ty();
        let rendered = self.render_ty(ty);
        let generics = self.cx.defs.get(def).generics().to_vec();
        let generics_rendered = self.generics_list(&generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    pub(crate) fn describe_def(&mut self, def: DefId) -> String {
        match &self.cx.defs.get(def).kind {
            DefKind::Fn(_) => self.describe_fn_item(def),
            DefKind::Param(_) => {
                let symbol = self.def_display_symbol(def);
                let ty = self.render_def_type(def);
                format!("{symbol}: {ty}")
            }
            DefKind::Local(_) => {
                let symbol = self.def_display_symbol(def);
                let ty = self.render_def_type(def);
                format!("let {symbol}: {ty}")
            }
            DefKind::TyAlias(_) => {
                format!("type {}", self.alias_symbol_with_generics(def))
            }
            DefKind::Mod(_) => format!("mod {}", self.def_display_symbol(def)),
            DefKind::GenericParam(_) => self.render_generic_param(DefIdOf::new_unchecked(def)),
            DefKind::Struct(_) | DefKind::Enum(_) | DefKind::Variant(_) | DefKind::Trait(_) => {
                self.render_def_type(def)
            }
        }
    }

    pub(crate) fn def_display_symbol(&mut self, def: DefId) -> String {
        let symbol = self.cx.defs.get(def).symbol;
        self.cx.symbols.resolve(symbol).to_owned()
    }

    pub(crate) fn alias_symbol_with_generics(&mut self, def: DefId) -> String {
        let symbol = self.def_display_symbol(def);
        let generics = self.cx.defs.get(def).generics().to_vec();
        let generics_rendered = self.generics_list(&generics);
        format!("{symbol}{generics_rendered}")
    }

    pub(crate) fn describe_fn_item(&mut self, def: DefId) -> String {
        let symbol = self.cx.defs.get(def).symbol;
        let symbol = self.cx.symbols.resolve(symbol).to_owned();

        let generics = self.cx.defs.get(def).generics().to_vec();
        let generics_rendered = self.generics_list(&generics);

        let DefKind::Fn(FnDef { params, .. }) = &self.cx.defs.get(def).kind else {
            unreachable!("describe_fn_item is only ever called for a DefKind::Fn def");
        };
        let param_symbols: Vec<String> = params.iter().map(|p| p.symbol.clone()).collect();

        let ty = self.cx.defs.get(def).ty();
        let resolved = self.cx.inf.resolve(ty);
        let Some(TyKind::Fn(param_types, output)) = self.cx.inf.ty(resolved).cloned() else {
            return self.render_def_type(def);
        };

        let params: Vec<String> = param_types
            .iter()
            .enumerate()
            .map(|(i, &ty)| match param_symbols.get(i) {
                Some(symbol) if symbol == ast::SELF_PARAM => symbol.clone(),
                Some(symbol) => format!("{symbol}: {}", self.render_ty(ty)),
                None => self.render_ty(ty),
            })
            .collect();

        let output_rendered = self.render_ty(output);
        format!(
            "fn {symbol}{generics_rendered}({}) -> {output_rendered}",
            params.join(", ")
        )
    }
}

pub(crate) fn resolved_kind(cx: &mut TypeCheckContext<'_>, ty: TyId) -> Option<TyKind> {
    let resolved = cx.inf.resolve(ty);
    cx.inf.ty(resolved).cloned()
}

pub(crate) fn declared_def(
    cx: &TypeCheckContext<'_>,
    scope: ScopeId,
    namespace: Namespace,
    symbol: &str,
) -> Option<DefId> {
    let symbol = cx.symbols.get(symbol)?;
    cx.scopes.lookup(scope, symbol, namespace)
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
    let def = cx
        .resolve_path_to_type(&target)
        .expect("Foo should resolve");

    let ty_val = ty(&mut cx.symbols, "Foo");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(
        resolved_kind(&mut cx, t),
        Some(TyKind::Struct(DefIdOf::new_unchecked(def), vec![]))
    );
}

#[test]
fn lower_ty_path_resolves_a_declared_enum_by_nominal_identity() {
    let mut cx = resolve("enum Foo { Bar }");
    let target = path(&mut cx.symbols, &["Foo"]);
    let def = cx
        .resolve_path_to_enum(&target)
        .expect("Foo should resolve");

    let ty_val = ty(&mut cx.symbols, "Foo");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(resolved_kind(&mut cx, t), Some(TyKind::Enum(def, vec![])));
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
    let def = cx
        .resolve_path_to_type(&target)
        .expect("m::Foo should resolve");

    let ty_val = ty(&mut cx.symbols, "m::Foo");
    let t = cx.lower_ty(&ty_val);
    assert_eq!(
        resolved_kind(&mut cx, t),
        Some(TyKind::Struct(DefIdOf::new_unchecked(def), vec![]))
    );
}

#[test]
fn target_implements_matches_when_the_query_args_unify_with_the_impls_trait_args() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        impl Into<bool> for bool {
            fn into() -> bool { true }
        }
    "#};
    let mut cx = resolve_and_lower(source);
    assert!(cx.diagnostics.is_empty());

    let into_def =
        declared_def(&cx, cx.current_scope, Namespace::Type, "Into").expect("Into should resolve");
    let (trait_def, _) = cx
        .trait_def_scope(into_def)
        .expect("Into should be a trait");

    let bool_ty = cx.ty(TyKind::Bool);
    assert!(cx.target_implements(bool_ty, trait_def, &[bool_ty]));
}

#[test]
fn target_implements_does_not_match_when_the_query_args_dont_unify() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        impl Into<bool> for bool {
            fn into() -> bool { true }
        }
    "#};
    let mut cx = resolve_and_lower(source);
    assert!(cx.diagnostics.is_empty());

    let into_def =
        declared_def(&cx, cx.current_scope, Namespace::Type, "Into").expect("Into should resolve");
    let (trait_def, _) = cx
        .trait_def_scope(into_def)
        .expect("Into should be a trait");

    let bool_ty = cx.ty(TyKind::Bool);
    let string_ty = cx.ty(TyKind::Str);
    assert!(!cx.target_implements(bool_ty, trait_def, &[string_ty]));
}

#[test]
fn target_implements_picks_the_matching_candidate_among_several_impls() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        impl Into<bool> for bool {
            fn into() -> bool { true }
        }
        impl Into<int> for bool {
            fn into() -> int { 0 }
        }
    "#};
    let mut cx = resolve_and_lower(source);
    assert!(cx.diagnostics.is_empty());

    let into_def =
        declared_def(&cx, cx.current_scope, Namespace::Type, "Into").expect("Into should resolve");
    let (trait_def, _) = cx
        .trait_def_scope(into_def)
        .expect("Into should be a trait");

    let bool_ty = cx.ty(TyKind::Bool);
    let int_ty = cx.ty(TyKind::Int);
    let string_ty = cx.ty(TyKind::Str);
    assert!(cx.target_implements(bool_ty, trait_def, &[bool_ty]));
    assert!(cx.target_implements(bool_ty, trait_def, &[int_ty]));
    assert!(!cx.target_implements(bool_ty, trait_def, &[string_ty]));
}

#[test]
fn target_implements_matches_via_a_blanket_impl_with_no_other_impls_for_the_target() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        impl<T> Into<T> for T {
            fn into() -> T { 0 }
        }
    "#};
    let mut cx = resolve_and_lower(source);
    assert!(cx.diagnostics.is_empty());

    let into_def =
        declared_def(&cx, cx.current_scope, Namespace::Type, "Into").expect("Into should resolve");
    let (trait_def, _) = cx
        .trait_def_scope(into_def)
        .expect("Into should be a trait");

    let int_ty = cx.ty(TyKind::Int);
    assert!(cx.target_implements(int_ty, trait_def, &[int_ty]));
}

#[test]
fn target_implements_does_not_match_a_blanket_impl_when_the_query_isnt_reflexive() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        impl<T> Into<T> for T {
            fn into() -> T { 0 }
        }
    "#};
    let mut cx = resolve_and_lower(source);
    assert!(cx.diagnostics.is_empty());

    let into_def =
        declared_def(&cx, cx.current_scope, Namespace::Type, "Into").expect("Into should resolve");
    let (trait_def, _) = cx
        .trait_def_scope(into_def)
        .expect("Into should be a trait");

    let bool_ty = cx.ty(TyKind::Bool);
    let int_ty = cx.ty(TyKind::Int);
    assert!(!cx.target_implements(bool_ty, trait_def, &[int_ty]));
}

#[test]
fn target_implements_matches_via_either_a_concrete_impl_or_a_blanket_impl() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        impl Into<bool> for bool {
            fn into() -> bool { true }
        }
        impl<T> Into<T> for T {
            fn into() -> T { 0 }
        }
    "#};
    let mut cx = resolve_and_lower(source);
    assert!(cx.diagnostics.is_empty());

    let into_def =
        declared_def(&cx, cx.current_scope, Namespace::Type, "Into").expect("Into should resolve");
    let (trait_def, _) = cx
        .trait_def_scope(into_def)
        .expect("Into should be a trait");

    let bool_ty = cx.ty(TyKind::Bool);
    let string_ty = cx.ty(TyKind::Str);
    assert!(cx.target_implements(bool_ty, trait_def, &[bool_ty]));
    assert!(cx.target_implements(string_ty, trait_def, &[string_ty]));
    assert!(!cx.target_implements(bool_ty, trait_def, &[string_ty]));
}

pub(crate) fn check_all_frozen(source: &str) -> CheckedProgram {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");
    CheckedProgram::check(&items, state.0)
}
