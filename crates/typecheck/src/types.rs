//! Defines the [`Type`] enum, representing a type in the final
//! result of the type checking process. This enum is also
//! responsible for lowering from the tys used internally
//! throughout the entire type checking process, to the final
//! Type variants.

use ast::{Pat, PatKind};
use intern::Interner;

use crate::defs::{Defs, EnumDef, GenericParamDef, StructDef, TraitDef};
use crate::inference::{InferenceTable, TyId, VarId};
use crate::{DefIdOf, TypeCheckContext};

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
    Struct(DefIdOf<StructDef>, Vec<TyId>),
    Enum(DefIdOf<EnumDef>, Vec<TyId>),
    TraitObject(DefIdOf<TraitDef>, Vec<TyId>),
    Generic(DefIdOf<GenericParamDef>),
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
        TypeResolver {
            inf: &mut self.inf,
            defs: &self.defs,
            names: &self.symbols,
        }
        .resolve(ty)
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
    Named(String, Vec<Type>),
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
            Type::Generic(name) => name.clone(),
            Type::Named(name, args) if args.is_empty() => name.clone(),
            Type::Named(name, args) => {
                let args: Vec<String> = args.iter().map(Type::render).collect();
                format!("{}<{}>", name, args.join(", "))
            }
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

/// Converts intermediate `ty`s used throughout type checking into
/// fully-resolved [`Type`]s. This essentially freezes each ty,
/// replacing any still unbound inference variables with
/// [`Type::Unresolved`].
///
/// The resulting `Type`s are recursive and can be read entirely on
/// their own without needing to have access to names, defs,
/// generics, or the unification context.
pub(crate) struct TypeResolver<'a> {
    pub(crate) inf: &'a mut InferenceTable,
    pub(crate) defs: &'a Defs,
    pub(crate) names: &'a Interner,
}

impl TypeResolver<'_> {
    pub(crate) fn resolve(&mut self, ty: TyId) -> Type {
        let resolved = self.inf.resolve(ty);
        let Some(kind) = self.inf.ty(resolved).cloned() else {
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
            TyKind::Array(elem) => Type::Array(Box::new(self.resolve(elem))),
            TyKind::Tuple(args) => Type::Tuple(args.iter().map(|&arg| self.resolve(arg)).collect()),
            TyKind::Fn(params, output) => {
                let params = params.iter().map(|&arg| self.resolve(arg)).collect();
                let output = Box::new(self.resolve(output));
                Type::Fn(params, output)
            }
            TyKind::Struct(def, args) => {
                let name = self.defs.get(def.id()).symbol;
                let args = args.iter().map(|&arg| self.resolve(arg)).collect();
                Type::Named(self.names.resolve(name).to_owned(), args)
            }
            TyKind::Enum(def, args) => {
                let name = self.defs.get(def.id()).symbol;
                let args = args.iter().map(|&arg| self.resolve(arg)).collect();
                Type::Named(self.names.resolve(name).to_owned(), args)
            }
            TyKind::TraitObject(def, args) => {
                let name = self.defs.get(def.id()).symbol;
                let args = args.iter().map(|&arg| self.resolve(arg)).collect();
                Type::Named(self.names.resolve(name).to_owned(), args)
            }
            TyKind::Generic(id) => {
                let symbol = self.defs.generic_param_ref(id).name;
                let text = self.names.resolve(symbol);
                Type::Generic(text.to_owned())
            }
        }
    }
}
