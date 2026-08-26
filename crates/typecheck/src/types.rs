//! Defines the [`Type`] enum, representing a type in the final
//! result of the type checking process. This enum is also
//! responsible for lowering from the tys used internally
//! throughout the entire type checking process, to the final
//! Type variants.

use ast::{Pat, PatKind};
use intern::Interner;
use slotmap::SlotMap;

use crate::generic_names::GenericNames;
use crate::inference::{InferenceTable, TyId, VarId};
use crate::{Def, DefId, GenericId, TypeCheckContext};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TyKind {
    Var(VarId),
    Never,
    Int,
    Float,
    Bool,
    Str,
    Err,
    Array(TyId),
    Tuple(Vec<TyId>),
    Fn(Vec<TyId>, TyId),
    Struct(DefId),
    Enum(DefId),
    Generic(GenericId),
}

// Helper to convert a pattern into a string.
//
// NOTE currently uses `_` for any pattern which isnt a simple
// identifier.
pub(crate) fn pat_display_name(pat: &Pat, names: &Interner) -> String {
    match &pat.kind {
        PatKind::Ident(ident, _) => names.resolve(ident.symbol).to_owned(),
        _ => "_".to_owned(),
    }
}

impl<'ast> TypeCheckContext<'ast> {
    // Produces a final completely resolved type from an intermediate
    // ty. This is done once the entirety of the type checking
    // process is completed.
    pub(crate) fn resolved(&mut self, ty: TyId) -> Type {
        resolve_type(
            &mut self.inf,
            &self.defs,
            &self.symbols,
            &self.generic_names,
            ty,
        )
    }
}

/// A fully resolved type.
///
/// The entire type checking process operates on `ty`s which are only
/// used internally. `ty` is the live, mutable representation that
/// checking and inference actually works with, and only makes sense
/// in tys of a `UnificationContext`.
///
/// internal `ty`s are first lowered to `Types` once the checking
/// process is complete.
#[derive(Debug, Clone)]
pub(crate) enum Type {
    Never,
    Int,
    Float,
    Bool,
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
            Type::Never => "!".to_owned(),
            Type::Int => "int".to_owned(),
            Type::Float => "float".to_owned(),
            Type::Bool => "bool".to_owned(),
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

/// Converts an intermediate `ty` used throughout type checking
/// into a fully-resolved `Type`. This essentially freezes the ty,
/// replacing any still unbound inference variables with
/// [`Type::Unresolved`].
///
/// The resulting `Type` is recursive and can be read entirely on
/// its own without needing to have access to names, defs,
/// generics, or the unification context.
pub(crate) fn resolve_type(
    inf: &mut InferenceTable,
    defs: &SlotMap<DefId, Def>,
    names: &Interner,
    generic_names: &GenericNames,
    ty: TyId,
) -> Type {
    let resolved = inf.resolve(ty);
    let Some(kind) = inf.ty(resolved).cloned() else {
        return Type::Unresolved;
    };

    match kind {
        TyKind::Var(_) => Type::Unresolved,
        TyKind::Never => Type::Never,
        TyKind::Int => Type::Int,
        TyKind::Float => Type::Float,
        TyKind::Bool => Type::Bool,
        TyKind::Str => Type::Str,
        TyKind::Err => Type::Err,
        TyKind::Array(elem) => Type::Array(Box::new(resolve_type(
            inf,
            defs,
            names,
            generic_names,
            elem,
        ))),
        TyKind::Tuple(args) => Type::Tuple(
            args.iter()
                .map(|&arg| resolve_type(inf, defs, names, generic_names, arg))
                .collect(),
        ),
        TyKind::Fn(params, output) => {
            let params = params
                .iter()
                .map(|&arg| resolve_type(inf, defs, names, generic_names, arg))
                .collect();
            let output = Box::new(resolve_type(inf, defs, names, generic_names, output));
            Type::Fn(params, output)
        }
        TyKind::Struct(def) | TyKind::Enum(def) => {
            let name = defs[def].symbol;
            Type::Named(names.resolve(name).to_owned())
        }
        TyKind::Generic(id) => {
            let text = generic_names
                .get(&id)
                .cloned()
                .unwrap_or_else(|| "<generic>".to_owned());
            Type::Generic(text)
        }
    }
}
