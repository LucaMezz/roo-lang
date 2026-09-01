use ast::Item;
use chumsky::Parser;
use indoc::indoc;
use typecheck::*;

fn check(source: &str) -> CheckedProgram {
    let tokens = lexer::tokenize_all(source).expect("should lex");
    let mut state = parser::State::default();
    let items = parser::module()
        .parse_with_state(parser::input(tokens), &mut state)
        .into_result()
        .expect("should parse");
    let items: &'static [Box<Item>] = Vec::leak(items);
    let symbols = state.0;
    CheckedProgram::check(items, symbols)
}

#[test]
fn check_calling_an_annotated_non_fn_parameter_is_an_error() {
    let source = indoc! {r#"
        fn use_it(g: int) {
            g(1);
        }
    "#};

    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.message(), "expected a function, found `int`");
    assert_eq!(&source[d.span().start..d.span().end], "g");
}

#[test]
fn check_calling_a_locally_inferred_non_fn_value_is_an_error() {
    let source = indoc! {r#"
        fn use_it() {
            let g = 5;
            g(1);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected a function, found `int`");
}

#[test]
fn check_referencing_an_undefined_value_is_an_error() {
    let source = indoc! {r#"
        fn use_it() {
            let x = totally_undefined_symbol;
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
fn check_redeclaring_a_function_in_the_same_scope_is_an_error() {
    let source = "fn foo() -> int { 1 } fn foo() -> bool { true }";
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
fn check_redeclaring_a_module_in_the_same_scope_is_an_error() {
    let source = "mod m {} mod m {}";
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "the symbol `m` is defined multiple times"
    );
}

#[test]
fn check_duplicate_parameter_symbols_are_an_error() {
    let source = "fn use_it(x: int, x: bool) {}";
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "the symbol `x` is defined multiple times"
    );
}

#[test]
fn check_let_defs_can_shadow_each_other_freely() {
    let source = indoc! {r#"
        fn use_it() {
            let x = 1;
            let x = "now a string";
            let x = true;
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_a_let_def_can_shadow_a_parameter_of_the_same_symbol() {
    let source = indoc! {r#"
        fn use_it(x: int) {
            let x = "shadow the param";
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_self_application_is_a_cyclic_type_error() {
    let source = indoc! {r#"
        fn cyclic(x) {
            x(x)
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].level(), Level::Error);
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
}

#[test]
fn check_a_directly_self_referential_ty_alias_is_a_cyclic_type_error() {
    let source = "type Foo = (Foo, int);";
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].level(), Level::Error);
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
    assert_eq!(
        &source[diagnostics[0].span().start..diagnostics[0].span().end],
        "(Foo, int)"
    );
}

#[test]
fn check_a_mutually_recursive_ty_alias_pair_is_a_cyclic_type_error() {
    let source = indoc! {r#"
        type A = (B, int);
        type B = (A, int);
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "cyclic type of infinite size");
}

#[test]
fn check_a_generic_ty_alias_referencing_only_its_own_params_is_not_cyclic() {
    let source = "type Pair<T, U> = (T, U);";
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn lower_signatures_impl_missing_trait_items_is_an_error() {
    let source = indoc! {r#"
        trait Greet {
            fn hello(x: int) -> int;
            type Output;
        }
        struct Foo;
        impl Greet for Foo {
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "missing function `hello` from trait `Greet`"
    );
    assert_eq!(
        diagnostics[1].message(),
        "missing associated type `Output` from trait `Greet`"
    );
}

#[test]
fn lower_signatures_impl_with_every_trait_item_has_no_error() {
    let source = indoc! {r#"
        trait Greet {
            fn hello(x: int) -> int;
            type Output;
        }
        struct Foo;
        impl Greet for Foo {
            fn hello(x: int) -> int { x }
            type Output = int;
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn lower_signatures_impl_with_extra_items_beyond_the_trait_has_no_error() {
    let source = indoc! {r#"
        trait Greet {
            fn hello(x: int) -> int;
        }
        struct Foo;
        impl Greet for Foo {
            fn hello(x: int) -> int { x }
            fn extra() -> int { 0 }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn lower_signatures_impl_fn_with_a_mismatched_signature_is_an_error() {
    let source = indoc! {r#"
        trait Greet {
            fn hello(x: int) -> int;
        }
        struct Foo;
        impl Greet for Foo {
            fn hello(x: int) -> bool { true }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(
        d.message(),
        "expected `Fn(int) -> int`, found `Fn(int) -> bool`"
    );
    assert_eq!(&source[d.span().start..d.span().end], "hello");
    assert_eq!(d.related().len(), 1);
    let (related_span, related_message) = &d.related()[0];
    assert_eq!(&source[related_span.start..related_span.end], "hello");
    assert_eq!(related_message, "expected due to this");
}

#[test]
fn lower_signatures_impl_assoc_type_with_a_mismatched_ty_is_an_error() {
    let source = indoc! {r#"
        trait Greet {
            type Output = int;
        }
        struct Foo;
        impl Greet for Foo {
            type Output = bool;
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `bool`");
}

#[test]
fn lower_signatures_trait_item_can_reference_the_traits_own_generic_param() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn lower_signatures_impl_of_generic_trait_instantiates_the_traits_generics_before_matching() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        struct Foo;
        impl Into<bool> for Foo {
            fn into() -> bool { true }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn lower_signatures_impl_of_generic_trait_still_catches_a_real_mismatch() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into() -> K;
        }
        struct Foo;
        impl Into<bool> for Foo {
            fn into() -> String { "" }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "expected `Fn() -> bool`, found `Fn() -> String`"
    );
}

#[test]
fn lower_signatures_impl_trait_item_mismatch_substitutes_self_with_the_impls_self_ty() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into(self) -> K;
        }
        impl Into<bool> for bool {
            fn into() -> bool { true }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "`into` is missing a `self` parameter required by trait `Into`"
    );
    assert_eq!(
        diagnostics[1].message(),
        "expected `Fn(bool) -> bool`, found `Fn() -> bool`"
    );
}

#[test]
fn lower_signatures_impl_trait_item_replacing_self_with_a_same_typed_param_is_an_error() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into(self) -> K;
        }
        impl Into<bool> for bool {
            fn into(x: bool) -> bool { x }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "`into` is missing a `self` parameter required by trait `Into`"
    );
}

#[test]
fn lower_signatures_impl_trait_item_adding_an_unexpected_self_param_is_an_error() {
    let source = indoc! {r#"
        trait Make<K> {
            fn make() -> K;
        }
        impl Make<bool> for bool {
            fn make(self) -> bool { self }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "`make` has a `self` parameter, but trait `Make` does not declare one"
    );
    assert_eq!(
        diagnostics[1].message(),
        "expected `Fn() -> bool`, found `Fn(bool) -> bool`"
    );
}

#[test]
fn lower_fn_item_self_param_outside_impl_or_trait_is_an_error() {
    let source = "fn hello(self) -> int { 0 }";
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "`self` parameter is only valid inside an `impl` or `trait` block"
    );
}

#[test]
fn lower_fn_item_self_param_in_a_nested_fn_inside_a_free_fn_is_an_error() {
    let source = indoc! {r#"
        fn outer() {
            fn inner(self) -> int { 0 }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "`self` parameter is only valid inside an `impl` or `trait` block"
    );
}

#[test]
fn lower_signatures_impl_trait_item_with_matching_self_param_has_no_diagnostics() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into(self) -> K;
        }
        impl Into<bool> for bool {
            fn into(self) -> bool { self }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_trait_default_fn_body_is_type_checked() {
    let source = indoc! {r#"
        trait Greet {
            fn hello() -> int {
                true
            }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `bool`");
}

#[test]
fn check_all_impl_fn_body_is_type_checked() {
    let source = indoc! {r#"
        struct Foo;
        impl Foo {
            fn hello() -> int {
                true
            }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `bool`");
}

#[test]
fn check_all_method_call_checks_argument_types_against_the_signature() {
    let source = indoc! {r#"
        struct Point {
            x: int,
        }
        impl Point {
            fn set_x(self, x: int) -> int {
                x
            }
        }
        fn use_it() {
            let p = Point { x: 5 };
            p.set_x(true);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `bool`");
}

#[test]
fn check_all_method_call_on_a_generic_param_is_an_invalid_receiver() {
    let source = indoc! {r#"
        fn foo<T>(x: T) -> int {
            x.bar()
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert!(diagnostics[0].message().contains("no method can be called"));
}

#[test]
fn check_all_method_call_with_unknown_name_is_unresolved() {
    let source = indoc! {r#"
        struct Point {
            x: int,
        }
        impl Point {
            fn get_x(self) -> int {
                self.x
            }
        }
        fn use_it() {
            let p = Point { x: 5 };
            p.missing_method();
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "no method named `missing_method` found for type `Point`"
    );
}

#[test]
fn check_all_method_call_on_an_associated_fn_without_self_is_not_a_method() {
    let source = indoc! {r#"
        struct Point {
            x: int,
        }
        impl Point {
            fn new() -> Point {
                Point { x: 0 }
            }
        }
        fn use_it() {
            let p = Point::new();
            p.new();
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(
        diagnostics[0].message(),
        "associated function `new` on `Point` cannot be called with method syntax because it has no `self` parameter"
    );
}

#[test]
fn check_all_method_call_uses_explicit_turbofish_generic_args() {
    let source = indoc! {r#"
        struct Container {
            n: int,
        }
        impl Container {
            fn wrap<T>(self, x: T) -> T {
                x
            }
        }
        fn use_it() {
            let c = Container { n: 0 };
            c.wrap::<int>(true);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `bool`");
}

#[test]
fn check_all_self_struct_literal_still_checks_field_types() {
    let source = indoc! {r#"
        struct Foo {
            x: int,
        }
        impl Foo {
            fn make() -> Foo {
                Self { x: "wrong" }
            }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `String`");
}

#[test]
fn check_all_trait_impl_fn_body_is_type_checked() {
    let source = indoc! {r#"
        trait Greet {
            fn hello() -> int;
        }
        struct Foo;
        impl Greet for Foo {
            fn hello() -> int {
                true
            }
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `bool`");
}

#[test]
fn check_all_emphasizes_only_the_specific_conflicting_portion_of_a_compound_type() {
    let source = indoc! {r#"
        fn add_one(x: int) -> int {
            x
        }
        fn use_it() {
            let f: Fn(int) -> String = add_one;
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
    let source = indoc! {r#"
        fn add(a: int, b: int) {}
        fn main() {
            add("wrong", 5);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(&source[d.span().start..d.span().end], "\"wrong\"");
    assert_eq!(d.message(), "expected `int`, found `String`");
}

#[test]
fn check_all_call_mismatch_against_an_annotated_param_points_at_the_annotation() {
    let source = indoc! {r#"
        fn add(a: int, b: int) {}
        fn main() {
            add("wrong", 5);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.related().len(), 1, "{:#?}", d.related());
    let (span, message) = &d.related()[0];
    assert_eq!(&source[span.start..span.end], "int");
    assert_eq!(message, "expected due to this");
}

#[test]
fn check_all_call_mismatch_against_an_unannotated_param_has_no_expected_due_to_this_note() {
    let source = indoc! {r#"
        fn takes_something(x) {
            let y: int = x;
            x
        }
        fn use_it() {
            takes_something("wrong");
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
    let source = indoc! {r#"
        fn takes_something(x) {
            let y: int = x;
            x
        }
        fn use_it() {
            takes_something("wrong");
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(d.related().len(), 1, "{:#?}", d.related());
    let (span, message) = &d.related()[0];
    assert_eq!(&source[span.start..span.end], "int");
    assert_eq!(message, "expected `int` was inferred here");
}

#[test]
fn check_all_cites_where_an_unannotated_params_fn_shape_was_first_inferred() {
    let source = indoc! {r#"
        fn use_it(f) {
            f(1);
            let x: String = f;
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
    let source = indoc! {r#"
        fn identity<T>(x: T) -> T {
            x
        }
        fn use_it() {
            identity::<int>("wrong");
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
fn check_all_a_trait_bound_argument_can_be_satisfied_only_by_a_blanket_impl() {
    let source = indoc! {r#"
        trait Into<K> {
            fn into(self) -> K;
        }
        impl<T> Into<T> for T {
            fn into(self) -> T { self }
        }
        fn check(thing: Into<int>) {}
        fn use_it() {
            check(1);
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_trait_default_fn_body_with_no_error_has_no_diagnostics() {
    let source = indoc! {r#"
        trait Greet {
            fn hello() -> int {
                1
            }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_trait_fn_with_no_default_body_has_no_diagnostics() {
    let source = indoc! {r#"
        trait Greet {
            fn hello() -> int;
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_impl_fn_body_with_no_error_has_no_diagnostics() {
    let source = indoc! {r#"
        struct Foo;
        impl Foo {
            fn hello() -> int {
                1
            }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_method_call_resolves_through_the_receivers_impl() {
    let source = indoc! {r#"
        struct Point {
            x: int,
        }
        impl Point {
            fn get_x(self) -> int {
                self.x
            }
        }
        fn use_it() -> int {
            let p = Point { x: 5 };
            p.get_x()
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_method_call_instantiates_the_impls_own_generics_from_the_receiver() {
    let source = indoc! {r#"
        struct Wrapper<T> {
            value: T,
        }
        impl<T> Wrapper<T> {
            fn get(self) -> T {
                self.value
            }
        }
        fn use_it() -> int {
            let w = Wrapper { value: 5 };
            w.get()
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_self_type_annotation_resolves_inside_an_impl_fn_body() {
    let source = indoc! {r#"
        struct Foo;
        impl Foo {
            fn make(self) -> Foo {
                let x: Self = self;
                x
            }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_self_qualified_call_resolves_inside_an_impl_fn_body() {
    let source = indoc! {r#"
        struct Foo;
        impl Foo {
            fn make() -> Foo {
                Foo
            }
            fn remake() -> Foo {
                Self::make()
            }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_self_struct_literal_resolves_inside_an_impl_fn_body() {
    let source = indoc! {r#"
        struct Foo {
            x: int,
        }
        impl Foo {
            fn make() -> Foo {
                Self { x: 1 }
            }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_self_struct_literal_resolves_for_a_generic_struct() {
    let source = indoc! {r#"
        struct Pair<T> {
            a: T,
            b: T,
        }
        impl Pair<int> {
            fn zero() -> Pair<int> {
                Self { a: 0, b: 0 }
            }
        }
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}

#[test]
fn check_all_a_parameter_shadowing_a_siblings_symbol_is_not_treated_as_a_call_to_it() {
    let source = indoc! {r#"
        fn apply(f, x) {
            f(x)
        }
        fn f(x) {
            apply(x, x)
        }
    "#};
    let checked = check(source);

    let apply = checked
        .def_at(source.find("apply").unwrap())
        .expect("apply should resolve");
    assert_eq!(
        checked.render_def_type(apply),
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );

    let diagnostics = checked.diagnostics(Locale::EnAu);
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
fn check_all_a_let_def_shadowing_a_siblings_symbol_is_not_treated_as_a_call_to_it() {
    let source = indoc! {r#"
        fn apply(g, x) {
            let f = g;
            f(x)
        }
        fn f(x) {
            apply(x, x)
        }
    "#};
    let checked = check(source);

    let apply = checked
        .def_at(source.find("apply").unwrap())
        .expect("apply should resolve");
    assert_eq!(
        checked.render_def_type(apply),
        "<T, U> Fn(Fn(T) -> U, T) -> U"
    );

    let diagnostics = checked.diagnostics(Locale::EnAu);
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
fn check_all_call_keeps_checking_later_arguments_after_an_earlier_one_mismatches() {
    let source = indoc! {r#"
        fn add(a: int, b: int) {}
        fn main() {
            add("wrong1", "wrong2");
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
    let source = indoc! {r#"
        fn add(a: int, b: int) {}
        fn main() {
            add(1);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
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
    let source = indoc! {r#"
        fn add(a: int, b: int) {}
        fn main() {
            add(1, 2, 3);
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");

    let d = &diagnostics[0];
    assert_eq!(
        d.message(),
        "this function takes 2 arguments but 3 arguments were supplied"
    );
    assert_eq!(&source[d.span().start..d.span().end], "3");
}

#[test]
fn check_all_a_real_type_error_inside_a_cyclic_group_is_still_reported() {
    let source = indoc! {r#"
        fn ping3(x: int) {
            pong3(x)
        }
        fn pong3(y: int) {
            ping3("wrong")
        }
    "#};
    let checked = check(source);
    let diagnostics = checked.diagnostics(Locale::EnAu);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    assert_eq!(diagnostics[0].message(), "expected `int`, found `String`");
}

#[test]
fn check_all_a_nested_fns_deferred_generalization_is_actually_usable_at_two_types() {
    let source = indoc! {r#"
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
    "#};
    let checked = check(source);
    assert!(
        checked.diagnostics(Locale::EnAu).is_empty(),
        "{:#?}",
        checked.diagnostics(Locale::EnAu)
    );
}
