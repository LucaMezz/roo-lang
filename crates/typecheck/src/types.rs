//! Defines the [`Type`] enum, representing a type in the final
//! result of the type checking process. This enum is also
//! responsible for lowering from the terms used internally
//! throughout the entire type checking process, to the final
//! Type variants.

use ast::{Pat, PatKind, Span};
use slotmap::SlotMap;
use unify::{Term, UnificationContext};

use crate::generic_names::GenericNames;
use crate::{NameInterner, Symbol, SymbolId, TermId, TyCon, TypeCheckContext};

// Helper to convert a pattern into a string.
//
// NOTE currently uses `_` for any pattern which isnt a simple
// identifier.
pub(crate) fn pat_display_name(pat: &Pat) -> String {
    match &pat.kind {
        PatKind::Ident(ident, _) => ident.name.clone(),
        _ => "_".to_owned(),
    }
}

impl TypeCheckContext {
    // Produces a final completely resolved type from an intermediate
    // term. This is done once the entirety of the type checking
    // process is completed.
    pub(crate) fn resolved(&mut self, term: TermId) -> Type {
        resolve_type(
            &mut self.uni_cx,
            &self.symbols,
            &self.names,
            &self.generic_names,
            term,
        )
    }
}

/// A fully resolved type.
///
/// The entire type checking process operates on `Term`s which are only
/// used internally. `Term` is the live, mutable representation that
/// checking and inference actually works with, and only makes sense
/// in terms of a `UnificationContext`.
///
/// internal `Term`s are first lowered to `Types` once the checking
/// process is complete.
#[derive(Debug, Clone)]
pub(crate) enum Type {
    Any,
    Never,
    Int,
    Float,
    Bool,
    Char,
    Str,
    Err,
    Unresolved,
    Array(Box<Type>),
    Tuple(Vec<Type>),
    Fn(Vec<Type>, Box<Type>),
    Named(String),
    Generic(String),
}

impl Type {
    /// A string representation of the type.
    pub(crate) fn render(&self) -> String {
        match self {
            Type::Any => "any".to_owned(),
            Type::Never => "!".to_owned(),
            Type::Int => "int".to_owned(),
            Type::Float => "float".to_owned(),
            Type::Bool => "bool".to_owned(),
            Type::Char => "char".to_owned(),
            Type::Str => "String".to_owned(),
            Type::Err => "<error>".to_owned(),
            Type::Unresolved => "_".to_owned(),
            Type::Named(name) | Type::Generic(name) => name.clone(),
            Type::Array(elem) => format!("[{}]", elem.render()),
            Type::Tuple(elems) => {
                let elems: Vec<String> = elems.iter().map(Type::render).collect();
                format!("({})", elems.join(", "))
            }
            Type::Fn(params, output) => {
                let params: Vec<String> = params.iter().map(Type::render).collect();
                format!("Fn({}) -> {}", params.join(", "), output.render())
            }
        }
    }
}

impl diagnostics::ToArgValue for Type {
    fn to_arg_value(&self) -> diagnostics::ArgValue {
        diagnostics::ArgValue::Text(self.render())
    }
}

/// Converts an intermediate `Term` used throughout type checking
/// into a fully-resolved `Type`. This essentially freezes the Term,
/// replacing any still unbound inference variables with
/// [`Type::Unresolved`].
///
/// The resulting `Type` is recursive and can be read entirely on
/// its own without needing to have access to names, symbols,
/// generics, or the unification context.
pub(crate) fn resolve_type(
    uni_cx: &mut UnificationContext<TyCon, Span>,
    symbols: &SlotMap<SymbolId, Symbol>,
    names: &NameInterner,
    generic_names: &GenericNames,
    term: TermId,
) -> Type {
    let resolved = uni_cx.resolve(term);
    let Some(term) = uni_cx.term(resolved).cloned() else {
        return Type::Unresolved;
    };
    let (constructor, args) = match term {
        Term::Var(_) => return Type::Unresolved,
        Term::App { constructor, args } => (constructor, args),
    };

    match constructor {
        TyCon::Any => Type::Any,
        TyCon::Never => Type::Never,
        TyCon::Int => Type::Int,
        TyCon::Float => Type::Float,
        TyCon::Bool => Type::Bool,
        TyCon::Char => Type::Char,
        TyCon::Str => Type::Str,
        TyCon::Err => Type::Err,
        TyCon::Array => Type::Array(Box::new(resolve_type(
            uni_cx,
            symbols,
            names,
            generic_names,
            args[0],
        ))),
        TyCon::Tuple => Type::Tuple(
            args.iter()
                .map(|&arg| resolve_type(uni_cx, symbols, names, generic_names, arg))
                .collect(),
        ),
        TyCon::Fn => {
            let resolved_inputs = uni_cx.resolve(args[0]);
            let param_types: Vec<TermId> = match uni_cx.term(resolved_inputs).cloned() {
                Some(Term::App {
                    constructor: TyCon::Tuple,
                    args,
                }) => args,
                _ => Vec::new(),
            };
            let params = param_types
                .iter()
                .map(|&arg| resolve_type(uni_cx, symbols, names, generic_names, arg))
                .collect();
            let output = Box::new(resolve_type(uni_cx, symbols, names, generic_names, args[1]));
            Type::Fn(params, output)
        }
        TyCon::Struct(symbol) | TyCon::Enum(symbol) => {
            let name = symbols[symbol].name;
            let text = names
                .name(name)
                .cloned()
                .unwrap_or_else(|| "<unknown>".to_owned());
            Type::Named(text)
        }
        TyCon::Generic(id) => {
            let text = generic_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "<generic>".to_owned());
            Type::Generic(text)
        }
    }
}
