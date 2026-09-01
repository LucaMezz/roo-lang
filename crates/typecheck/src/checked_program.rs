use std::collections::HashMap;

use ast::{Item, SELF_PARAM};
use diagnostics::Diagnostic;
use intern::Interner;

use crate::defs::{DefKind, Defs, FnDef};
use crate::errors::{Locale, TypeCheckDiagnostic};
use crate::inference::{InferenceTable, TyId};
use crate::position_index::PositionIndex;
use crate::types::{Type, TypeResolver};
use crate::{DefId, TypeCheckContext};

impl<'ast> TypeCheckContext<'ast> {
    /// Constructs the final result of the type checking stage, which
    /// will be output to the client of this crate.
    ///
    /// The process of freezing the [`TypeCheckContext`] to arrive at
    /// the final checked program involves converting  all defs
    /// within the def table into frozen defs. This replaces
    /// interned symbol ids with the actual symbol strings, converts the
    /// DefKind to a FrozenDefKind, and resolves the `Ty`
    /// representing the type of the def to a frozen `Type`.
    ///
    /// TODO Make the CheckedProgram result produced by the
    /// TypeCheckContext actually preserve the module -> exported
    /// def structure, since the embedding API will need to allow
    /// for calling / getting defs of items within modules.
    fn freeze(mut self) -> CheckedProgram {
        let mut defs = HashMap::with_capacity(self.defs.len());
        for (id, def) in self.defs.iter() {
            let ty = match def.kind.ty() {
                Some(ty) => TypeResolver {
                    inf: &mut self.inf,
                    defs: &self.defs,
                    names: &self.symbols,
                }
                .resolve(ty),
                None => Type::Unresolved,
            };
            let symbol = self.symbols.resolve(def.symbol).to_owned();
            let generics = def
                .generics()
                .iter()
                .map(|&gid| {
                    let param = self.defs.generic_param_ref(gid);
                    let name = self.symbols.resolve(param.name).to_owned();
                    let bounds =
                        render_bounds(&param.bounds, &mut self.inf, &self.defs, &self.symbols);
                    FrozenGeneric { name, bounds }
                })
                .collect();
            let kind = match &def.kind {
                DefKind::Fn(FnDef { params, .. }) => FrozenDefKind::Fn {
                    param_symbols: params.iter().map(|p| p.symbol.clone()).collect(),
                },
                DefKind::GenericParam(param) => FrozenDefKind::GenericParam {
                    bounds: render_bounds(&param.bounds, &mut self.inf, &self.defs, &self.symbols),
                },
                DefKind::Param(_) => FrozenDefKind::Param,
                DefKind::Local(_) => FrozenDefKind::Local,
                DefKind::TyAlias(_) => FrozenDefKind::TyAlias,
                DefKind::Mod(_) => FrozenDefKind::Mod,
                DefKind::Struct(_) | DefKind::Enum(_) | DefKind::Variant(_) | DefKind::Trait(_) => {
                    FrozenDefKind::Other
                }
            };
            defs.insert(
                id,
                FrozenDef {
                    symbol,
                    kind,
                    ty,
                    generics,
                },
            );
        }

        CheckedProgram {
            diagnostics: self.diagnostics.into_vec(),
            positions: self.positions,
            defs,
        }
    }
}

struct FrozenDef {
    symbol: String,
    kind: FrozenDefKind,
    ty: Type,
    generics: Vec<FrozenGeneric>,
}

struct FrozenGeneric {
    name: String,
    bounds: Vec<String>,
}

impl FrozenGeneric {
    fn render(&self) -> String {
        if self.bounds.is_empty() {
            self.name.clone()
        } else {
            format!("{}: {}", self.name, self.bounds.join(" + "))
        }
    }
}

fn render_bounds(
    bounds: &[TyId],
    inf: &mut InferenceTable,
    defs: &Defs,
    names: &Interner,
) -> Vec<String> {
    let mut rendered = Vec::with_capacity(bounds.len());
    for &bound in bounds {
        rendered.push(
            TypeResolver {
                inf: &mut *inf,
                defs,
                names,
            }
            .resolve(bound)
            .render(),
        );
    }
    rendered
}

enum FrozenDefKind {
    Fn { param_symbols: Vec<String> },
    GenericParam { bounds: Vec<String> },
    Param,
    Local,
    TyAlias,
    Mod,
    Other,
}

/// The final result produced by the type checking process.
/// Also provides several functions to query the typed program.
pub struct CheckedProgram {
    diagnostics: Vec<TypeCheckDiagnostic>,
    positions: PositionIndex,
    defs: HashMap<DefId, FrozenDef>,
}

impl CheckedProgram {
    /// Creates a new [`CheckedProgram`] by performing the entire
    /// type checking process on the given items. Performs type
    /// inference and confirms all types are compatible, and then
    /// returns the [`CheckedProgram`] containing information
    /// about the discovered items and their defs and types.
    pub fn check(items: &[Box<Item>], symbols: Interner) -> CheckedProgram {
        let mut cx = TypeCheckContext::new(symbols);
        cx.resolve(items);
        cx.lower_signatures(items);
        cx.check(items);
        cx.freeze()
    }

    /// Gets all the diagnostics that were emitted during the
    /// type checking process, given a Locale.
    ///
    /// The actual diagnostic messages are stored in `Fluent`
    /// `.lft` files, so a [`Locale`] can be passed which will
    /// render the diagnostics with that locale.
    pub fn diagnostics(&self, locale: Locale) -> Vec<Diagnostic> {
        let catalog = crate::errors::catalog(locale);
        self.diagnostics.iter().map(|d| d.render(catalog)).collect()
    }

    /// Queries the checked program to see if there is a def
    /// associated with a specific offset in the source file.
    pub fn def_at(&self, offset: usize) -> Option<DefId> {
        self.positions.def_at(offset)
    }

    /// Queries the checked program to see if there is a concrete
    /// primitive type symbol at a certain offset in the source file.
    pub fn type_symbol_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    fn def(&self, def: DefId) -> &FrozenDef {
        &self.defs[&def]
    }

    /// Returns a string representation of the type associated with
    /// a given def.
    pub fn render_def_type(&self, def: DefId) -> String {
        let bind = self.def(def);
        let rendered = bind.ty.render();
        let generics_rendered = generics_list(&bind.generics);
        if generics_rendered.is_empty() {
            rendered
        } else {
            format!("{generics_rendered} {rendered}")
        }
    }

    /// Returns a string representation of a def.
    pub fn describe_def(&self, def: DefId) -> String {
        let bind = self.def(def);
        match &bind.kind {
            FrozenDefKind::Fn { param_symbols } => describe_fn_item(bind, param_symbols),
            FrozenDefKind::GenericParam { bounds } if bounds.is_empty() => bind.symbol.clone(),
            FrozenDefKind::GenericParam { bounds } => {
                format!("{}: {}", bind.symbol, bounds.join(" + "))
            }
            FrozenDefKind::Param => format!("{}: {}", bind.symbol, bind.ty.render()),
            FrozenDefKind::Local => format!("let {}: {}", bind.symbol, bind.ty.render()),
            FrozenDefKind::TyAlias => {
                format!("type {}", alias_symbol_with_generics(bind))
            }
            FrozenDefKind::Mod => format!("mod {}", bind.symbol),
            FrozenDefKind::Other => self.render_def_type(def),
        }
    }
}

/// Returns a string representation of a generic list.
fn generics_list(generics: &[FrozenGeneric]) -> String {
    if generics.is_empty() {
        return String::new();
    }
    let params: Vec<String> = generics.iter().map(FrozenGeneric::render).collect();
    format!("<{}>", params.join(", "))
}

/// Returns a string with the def symbol followed by a
/// generic list of the def.
fn alias_symbol_with_generics(bind: &FrozenDef) -> String {
    format!("{}{}", bind.symbol, generics_list(&bind.generics))
}

/// Returns a full string representation of the entire signature
/// of a function.
fn describe_fn_item(bind: &FrozenDef, param_symbols: &[String]) -> String {
    let generics_rendered = generics_list(&bind.generics);
    let Type::Fn(params, output) = &bind.ty else {
        return bind.ty.render();
    };
    let params: Vec<String> = params
        .iter()
        .enumerate()
        .map(|(i, ty)| match param_symbols.get(i) {
            Some(symbol) if symbol == SELF_PARAM => symbol.clone(),
            Some(symbol) => format!("{symbol}: {}", ty.render()),
            None => ty.render(),
        })
        .collect();
    format!(
        "fn {}{generics_rendered}({}) -> {}",
        bind.symbol,
        params.join(", "),
        output.render()
    )
}

#[cfg(test)]
mod tests {
    use crate::tests::*;

    #[test]
    fn def_at_finds_a_fns_own_declaration_and_every_later_reference_to_it() {
        let source = indoc! {r#"
            fn add_one(x: int) -> int {
                x
            }
            fn use_it() {
                add_one(1);
            }
        "#};
        let mut cx = check_all(source);

        let decl_offset = source.find("add_one").unwrap() + 2;
        let use_offset = source.rfind("add_one").unwrap() + 2;
        assert_ne!(
            decl_offset, use_offset,
            "test source should have two distinct occurrences"
        );

        let decl_def = cx
            .def_at(decl_offset)
            .expect("should resolve at the declaration");
        let use_def = cx
            .def_at(use_offset)
            .expect("should resolve at the call site");
        assert_eq!(decl_def, use_def);
        assert_eq!(cx.renderer().render_def_type(decl_def), "Fn(int) -> int");
    }

    #[test]
    fn def_at_finds_a_parameter_at_both_its_declaration_and_its_use_in_the_body() {
        let source = "fn add_one(x: int) -> int { x }";
        let mut cx = check_all(source);

        let param_decl = source.find("x:").unwrap();
        let param_use = source.rfind('x').unwrap();
        assert_ne!(param_decl, param_use);

        let decl_def = cx
            .def_at(param_decl)
            .expect("should resolve at the parameter");
        let use_def = cx
            .def_at(param_use)
            .expect("should resolve at the body reference");
        assert_eq!(decl_def, use_def);
        assert_eq!(cx.renderer().render_def_type(decl_def), "int");
    }

    #[test]
    fn def_at_finds_a_let_defs_inferred_type() {
        let source = indoc! {r#"
            fn use_it() {
                let n = 1;
            }
        "#};
        let mut cx = check_all(source);
        let offset = source.find("let n").unwrap() + "let ".len();
        let def = cx.def_at(offset).expect("should resolve at the let def");
        assert_eq!(cx.renderer().render_def_type(def), "int");
    }

    #[test]
    fn def_at_finds_a_struct_symbol_at_its_declaration_and_in_a_type_annotation() {
        let source = indoc! {r#"
            struct Point {
                x: int,
            }
            fn use_it(p: Point) {}
        "#};
        let cx = check_all(source);

        let decl_offset = source.find("Point").unwrap();
        let use_offset = source.rfind("Point").unwrap();
        assert_ne!(decl_offset, use_offset);

        let decl_def = cx
            .def_at(decl_offset)
            .expect("should resolve at the struct decl");
        let use_def = cx
            .def_at(use_offset)
            .expect("should resolve at the annotation");
        assert_eq!(decl_def, use_def);
    }

    #[test]
    fn def_at_finds_a_generic_param_at_its_declaration_and_in_the_signature() {
        let source = "fn identity<T>(x: T) -> T { x }";
        let cx = check_all(source);

        let decl_offset = source.find('T').unwrap();
        let param_ty_offset = source.find("x: T").unwrap() + 3;
        let ret_ty_offset = source.rfind('T').unwrap();

        let decl_def = cx.def_at(decl_offset).expect("should resolve at <T>");
        let param_def = cx.def_at(param_ty_offset).expect("should resolve at x: T");
        let ret_def = cx.def_at(ret_ty_offset).expect("should resolve at -> T");
        assert_eq!(decl_def, param_def);
        assert_eq!(decl_def, ret_def);
    }

    #[test]
    fn def_at_is_none_between_identifiers() {
        let source = "fn add_one(x: int) -> int { x }";
        let cx = check_all(source);
        let space_offset = source.find(") ->").unwrap();
        assert_eq!(cx.def_at(space_offset), None);
    }

    #[test]
    fn describe_def_a_fn_item_uses_source_declaration_syntax_with_param_symbols() {
        let source = "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U { f(g(x)) }";
        let mut cx = check_all(source);
        let offset = source.find("compose").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(
            cx.renderer().describe_def(def),
            "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U"
        );
    }

    #[test]
    fn describe_def_a_fn_item_shows_the_bounds_on_its_generic_params() {
        let source = indoc! {r#"
            trait Show { fn show() -> int; }
            trait Eq { fn eq() -> bool; }
            fn dump<T: Show + Eq, U: Show>(a: T, b: U) -> T { a }
        "#};
        let mut cx = check_all(source);
        let offset = source.find("dump").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(
            cx.renderer().describe_def(def),
            "fn dump<T: Show + Eq, U: Show>(a: T, b: U) -> T"
        );
    }

    #[test]
    fn describe_def_a_generic_param_shows_its_own_bounds() {
        let source = indoc! {r#"
            trait Show { fn show() -> int; }
            fn dump<T: Show>(x: T) -> T { x }
        "#};
        let mut cx = check_all(source);
        let offset = source.find("T: Show").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the generic param");
        assert_eq!(cx.renderer().describe_def(def), "T: Show");
    }

    #[test]
    fn describe_def_a_parameter_is_prefixed_with_its_own_symbol() {
        let source = "fn add_one(x: int) -> int { x }";
        let mut cx = check_all(source);
        let offset = source.find("x:").unwrap();
        let def = cx.def_at(offset).expect("should resolve at the parameter");
        assert_eq!(cx.renderer().describe_def(def), "x: int");
    }

    #[test]
    fn describe_def_a_self_parameter_hovered_directly_still_shows_its_type() {
        let source = indoc! {r#"
            struct Foo;
            impl Foo {
                fn hello(self) -> bool { true }
            }
        "#};
        let mut cx = check_all(source);
        let offset = source.find("self)").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the self parameter");
        assert_eq!(cx.renderer().describe_def(def), "self: Foo");
    }

    #[test]
    fn describe_def_a_fn_item_with_a_self_param_omits_its_type_in_the_signature() {
        let source = indoc! {r#"
            struct Foo;
            impl Foo {
                fn hello(self, n: int) -> bool { true }
            }
        "#};
        let mut cx = check_all(source);
        let offset = source.find("hello").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(
            cx.renderer().describe_def(def),
            "fn hello(self, n: int) -> bool"
        );
    }

    #[test]
    fn describe_def_a_let_def_is_prefixed_with_let_and_its_own_symbol() {
        let source = indoc! {r#"
            fn use_it() {
                let n = 1;
            }
        "#};
        let mut cx = check_all(source);
        let offset = source.find("let n").unwrap() + "let ".len();
        let def = cx.def_at(offset).expect("should resolve at the let def");
        assert_eq!(cx.renderer().describe_def(def), "let n: int");
    }

    #[test]
    fn describe_def_a_higher_order_parameter_keeps_the_bare_fn_type_syntax() {
        let source = "fn apply<T, U>(f: Fn(T) -> U, x: T) -> U { f(x) }";
        let mut cx = check_all(source);
        let offset = source.find("f:").unwrap();
        let def = cx.def_at(offset).expect("should resolve at the parameter");
        assert_eq!(cx.renderer().describe_def(def), "f: Fn(T) -> U");
    }

    #[test]
    fn describe_def_a_ty_alias_declaration_shows_the_type_keyword() {
        let source = "type Pair<T, U> = (T, U);";
        let mut cx = check_all(source);
        let offset = source.find("Pair").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the alias's own symbol");
        assert_eq!(cx.renderer().describe_def(def), "type Pair<T, U>");
    }

    #[test]
    fn describe_def_a_ty_alias_reference_also_shows_the_type_keyword() {
        let source = indoc! {r#"
            type Pair<T, U> = (T, U);
            fn make_pair<T, U>(a: T, b: U) -> Pair<T, U> {
                (a, b)
            }
        "#};
        let mut cx = check_all(source);
        let offset = source.rfind("Pair").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the return-type reference");
        assert_eq!(cx.renderer().describe_def(def), "type Pair<T, U>");
    }

    #[test]
    fn describe_def_a_mod_declaration_shows_the_mod_keyword() {
        let source = "mod example { fn foo() {} }";
        let mut cx = check_all(source);
        let offset = source.find("example").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the module's own symbol");
        assert_eq!(cx.renderer().describe_def(def), "mod example");
    }

    #[test]
    fn describe_def_a_mod_reference_also_shows_the_mod_keyword() {
        let source = indoc! {r#"
            mod outer {
                mod inner {
                    fn foo() {}
                }
            }
            use outer::inner;
        "#};
        let mut cx = check_all(source);
        let offset = source.rfind("inner").unwrap();
        let def = cx
            .def_at(offset)
            .expect("should resolve at the use path's reference to the module");
        assert_eq!(cx.renderer().describe_def(def), "mod inner");
    }

    #[test]
    fn type_symbol_at_finds_every_literal_kinds_own_type() {
        let source = indoc! {r#"
            fn use_it() {
                let a = 1;
                let b = 1.5;
                let d = true;
                let e = "hi";
            }
        "#};
        let cx = check_all(source);
        assert_eq!(cx.type_symbol_at(source.find('1').unwrap()), Some("int"));
        assert_eq!(
            cx.type_symbol_at(source.find("1.5").unwrap()),
            Some("float")
        );
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
        let source = indoc! {r#"
            fn identity<T>(x: T) -> T {
                x
            }
            fn use_it() {
                identity::<int>(1);
            }
        "#};
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
    fn freeze_diagnostics_survive_the_freeze_unchanged() {
        let source = "fn use_it(g: int) { g(1); }";
        let frozen = check_all_frozen(source);
        let diagnostics = frozen.diagnostics(Locale::EnUs);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].message(), "expected a function, found `int`");
    }

    #[test]
    fn freeze_render_def_type_a_generic_fn() {
        let source = "fn identity<T>(x: T) -> T { x }";
        let frozen = check_all_frozen(source);
        let offset = source.find("identity").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(frozen.render_def_type(def), "<T> Fn(T) -> T");
    }

    #[test]
    fn freeze_render_def_type_a_generic_fn_with_bounds() {
        let source = indoc! {r#"
            trait Show { fn show() -> int; }
            fn identity<T: Show>(x: T) -> T { x }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("identity").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(frozen.render_def_type(def), "<T: Show> Fn(T) -> T");
    }

    #[test]
    fn freeze_render_def_type_a_struct_parameter() {
        let source = indoc! {r#"
            struct Point {
                x: int,
            }
            fn use_it(p: Point) {}
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("p: Point").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the parameter");
        assert_eq!(frozen.render_def_type(def), "Point");
    }

    #[test]
    fn freeze_render_def_type_a_mutually_recursive_pair_generalized_together() {
        let source = indoc! {r#"
            fn apply(f, x) {
                f(x)
            }
            fn f(x) {
                apply(x, x)
            }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("apply").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at apply's own symbol");
        assert_eq!(frozen.render_def_type(def), "<T, U> Fn(Fn(T) -> U, T) -> U");
    }

    #[test]
    fn freeze_describe_def_a_fn_item_uses_source_declaration_syntax_with_param_symbols() {
        let source = "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U { f(g(x)) }";
        let frozen = check_all_frozen(source);
        let offset = source.find("compose").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(
            frozen.describe_def(def),
            "fn compose<T, U, V>(f: Fn(T) -> U, g: Fn(V) -> T, x: V) -> U"
        );
    }

    #[test]
    fn freeze_describe_def_a_fn_item_shows_the_bounds_on_its_generic_params() {
        let source = indoc! {r#"
            trait Show { fn show() -> int; }
            trait Eq { fn eq() -> bool; }
            fn dump<T: Show + Eq, U: Show>(a: T, b: U) -> T { a }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("dump").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(
            frozen.describe_def(def),
            "fn dump<T: Show + Eq, U: Show>(a: T, b: U) -> T"
        );
    }

    #[test]
    fn freeze_describe_def_a_generic_param_shows_its_own_bounds() {
        let source = indoc! {r#"
            trait Show { fn show() -> int; }
            fn dump<T: Show>(x: T) -> T { x }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("T: Show").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the generic param");
        assert_eq!(frozen.describe_def(def), "T: Show");
    }

    #[test]
    fn freeze_describe_def_a_parameter_is_prefixed_with_its_own_symbol() {
        let source = "fn add_one(x: int) -> int { x }";
        let frozen = check_all_frozen(source);
        let offset = source.find("x:").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the parameter");
        assert_eq!(frozen.describe_def(def), "x: int");
    }

    #[test]
    fn freeze_describe_def_a_self_parameter_hovered_directly_still_shows_its_type() {
        let source = indoc! {r#"
            struct Foo;
            impl Foo {
                fn hello(self) -> bool { true }
            }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("self)").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the self parameter");
        assert_eq!(frozen.describe_def(def), "self: Foo");
    }

    #[test]
    fn freeze_describe_def_a_fn_item_with_a_self_param_omits_its_type_in_the_signature() {
        let source = indoc! {r#"
            struct Foo;
            impl Foo {
                fn hello(self, n: int) -> bool { true }
            }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("hello").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the fn's own symbol");
        assert_eq!(frozen.describe_def(def), "fn hello(self, n: int) -> bool");
    }

    #[test]
    fn freeze_describe_def_a_let_def_is_prefixed_with_let_and_its_own_symbol() {
        let source = indoc! {r#"
            fn use_it() {
                let n = 1;
            }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.find("let n").unwrap() + "let ".len();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the let def");
        assert_eq!(frozen.describe_def(def), "let n: int");
    }

    #[test]
    fn freeze_describe_def_a_higher_order_parameter_keeps_the_bare_fn_type_syntax() {
        let source = "fn apply<T, U>(f: Fn(T) -> U, x: T) -> U { f(x) }";
        let frozen = check_all_frozen(source);
        let offset = source.find("f:").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the parameter");
        assert_eq!(frozen.describe_def(def), "f: Fn(T) -> U");
    }

    #[test]
    fn freeze_describe_def_a_ty_alias_declaration_shows_the_type_keyword() {
        let source = "type Pair<T, U> = (T, U);";
        let frozen = check_all_frozen(source);
        let offset = source.find("Pair").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the alias's own symbol");
        assert_eq!(frozen.describe_def(def), "type Pair<T, U>");
    }

    #[test]
    fn freeze_describe_def_a_ty_alias_reference_also_shows_the_type_keyword() {
        let source = indoc! {r#"
            type Pair<T, U> = (T, U);
            fn make_pair<T, U>(a: T, b: U) -> Pair<T, U> {
                (a, b)
            }
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.rfind("Pair").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the return-type reference");
        assert_eq!(frozen.describe_def(def), "type Pair<T, U>");
    }

    #[test]
    fn freeze_describe_def_a_mod_declaration_shows_the_mod_keyword() {
        let source = "mod example { fn foo() {} }";
        let frozen = check_all_frozen(source);
        let offset = source.find("example").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the module's own symbol");
        assert_eq!(frozen.describe_def(def), "mod example");
    }

    #[test]
    fn freeze_describe_def_a_mod_reference_also_shows_the_mod_keyword() {
        let source = indoc! {r#"
            mod outer {
                mod inner {
                    fn foo() {}
                }
            }
            use outer::inner;
        "#};
        let frozen = check_all_frozen(source);
        let offset = source.rfind("inner").unwrap();
        let def = frozen
            .def_at(offset)
            .expect("should resolve at the use path's reference to the module");
        assert_eq!(frozen.describe_def(def), "mod inner");
    }

    #[test]
    fn freeze_type_symbol_at_still_finds_a_literals_type() {
        let source = "fn use_it() { 1; }";
        let frozen = check_all_frozen(source);
        let offset = source.find('1').unwrap();
        assert_eq!(frozen.type_symbol_at(offset), Some("int"));
    }
}
