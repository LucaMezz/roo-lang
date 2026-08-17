use std::collections::HashMap;

use ast::visit::{Visitor, Walkable};
use ast::{Fn, FnRetTy, FnTy, Item, ItemKind, ModKind, Path, Span, Ty, TyKind, VariantData};
use slotmap::SlotMap;
use unify::{TermId, UnificationContext, term};

mod call_graph;
mod check;
mod checked_program;
mod errors;
mod generic_names;
mod polymorphism;
mod position_index;
mod render;

use check::Checker;
use errors::Diagnostics;
use generic_names::GenericNames;
use position_index::PositionIndex;

pub use checked_program::CheckedProgram;
pub use diagnostics::{Diagnostic, Level};
pub use errors::Locale;

#[derive(Debug, Clone, PartialEq)]
enum TyCon {
    Any,

    Never,

    Int,
    Float,
    Bool,
    Char,
    Str,

    Fn,
    Array,
    Tuple,

    Struct(SymbolId),
    Enum(SymbolId),

    Generic(GenericId),

    Err,
}

slotmap::new_key_type! {
    pub struct ScopeId;

    pub struct SymbolId;

    pub struct GenericId;
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct NameId(usize);

#[derive(Clone, Copy)]
enum Namespace {
    Type,

    Value,
}

struct Scope {
    parent: Option<ScopeId>,

    types: HashMap<NameId, SymbolId>,

    values: HashMap<NameId, SymbolId>,
}

struct Symbol {
    name: NameId,

    kind: SymbolKind,

    ty: TermId,

    generics: Vec<GenericId>,

    declared_at: Span,
}

struct FnSymbol {
    scope: ScopeId,

    param_spans: Vec<Option<Span>>,

    param_names: Vec<String>,
}

enum SymbolKind {
    Struct,
    Enum,
    Variant,
    Trait,
    TyAlias(ScopeId),
    Mod(ScopeId),
    Fn(FnSymbol),
    Local,
    Param,
    GenericParam,
}

#[derive(Clone, Copy)]
enum PatDeclKind {
    Param,
    Let,
}

impl PatDeclKind {
    fn symbol_kind(self) -> SymbolKind {
        match self {
            PatDeclKind::Param => SymbolKind::Param,
            PatDeclKind::Let => SymbolKind::Local,
        }
    }
}

impl SymbolKind {
    fn namespace(&self) -> Namespace {
        match self {
            SymbolKind::Struct
            | SymbolKind::Enum
            | SymbolKind::Trait
            | SymbolKind::TyAlias(_)
            | SymbolKind::GenericParam
            | SymbolKind::Mod(_) => Namespace::Type,
            SymbolKind::Variant | SymbolKind::Fn(_) | SymbolKind::Local | SymbolKind::Param => {
                Namespace::Value
            }
        }
    }
}

struct NameInterner {
    strings: Vec<String>,
    ids: HashMap<String, NameId>,
}

impl NameInterner {
    pub fn new() -> Self {
        Self {
            strings: vec![],
            ids: HashMap::new(),
        }
    }

    pub fn id(&mut self, string: &str) -> NameId {
        if let Some(id) = self.ids.get(string) {
            return *id;
        }

        let id = NameId(self.strings.len());
        self.strings.push(string.to_owned());
        self.ids.insert(string.to_owned(), id);
        id
    }

    pub fn name(&self, id: NameId) -> Option<&String> {
        self.strings.get(id.0)
    }
}

struct TypeCheckContext {
    uni_cx: UnificationContext<TyCon, Span>,

    names: NameInterner,

    symbols: SlotMap<SymbolId, Symbol>,

    scopes: SlotMap<ScopeId, Scope>,

    generic_ids: SlotMap<GenericId, ()>,

    generic_names: GenericNames,

    current_scope: ScopeId,

    checking_stack: Vec<SymbolId>,

    diagnostics: Diagnostics,

    positions: PositionIndex,
}

impl Default for TypeCheckContext {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeCheckContext {
    fn new() -> Self {
        let mut scopes = SlotMap::with_key();
        let root = scopes.insert(Scope {
            parent: None,
            types: HashMap::new(),
            values: HashMap::new(),
        });

        Self {
            uni_cx: UnificationContext::with_wildcards(vec![TyCon::Any, TyCon::Err, TyCon::Never]),

            names: NameInterner::new(),
            scopes,
            generic_ids: SlotMap::with_key(),
            generic_names: GenericNames::new(),
            symbols: SlotMap::with_key(),
            current_scope: root,
            checking_stack: Vec::new(),
            diagnostics: Diagnostics::default(),
            positions: PositionIndex::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn diagnostics(&self) -> Vec<Diagnostic> {
        let catalog = errors::catalog(errors::Locale::EnUs);
        self.diagnostics
            .as_slice()
            .iter()
            .map(|d| d.render(catalog))
            .collect()
    }

    fn record_path_reference(&mut self, path: &Path, symbol: SymbolId) {
        if let Some(segment) = path.segments.last() {
            self.positions.record_symbol(segment.ident.span, symbol);
        }
    }

    #[cfg(test)]
    pub(crate) fn symbol_at(&self, offset: usize) -> Option<SymbolId> {
        self.positions.symbol_at(offset)
    }

    #[cfg(test)]
    pub(crate) fn type_name_at(&self, offset: usize) -> Option<&'static str> {
        self.positions.type_name_at(offset)
    }

    fn resolve(&mut self, items: &[Box<Item>]) {
        let mut resolver = Resolver { cx: self };
        for item in items {
            resolver.visit_item(item);
        }
    }

    fn lower_signatures(&mut self, items: &[Box<Item>]) {
        let mut lowerer = SignatureLowerer { cx: self };
        for item in items {
            lowerer.visit_item(item);
        }
    }

    fn check(&mut self, items: &[Box<Item>]) {
        let mut checker = Checker::new(self);
        let items: Vec<&Item> = items.iter().map(Box::as_ref).collect();
        checker.check_items(&items);
    }

    fn resolve_path(&mut self, path: &Path, namespace: Namespace) -> Option<SymbolId> {
        let mut segments = path.segments.iter().peekable();

        let first = segments.next()?;
        let name = self.names.id(&first.ident.name);

        let ns = if segments.peek().is_some() {
            Namespace::Type
        } else {
            namespace
        };

        let mut symbol = self.lookup_up_scope_chain(self.current_scope, name, ns)?;

        while let Some(segment) = segments.next() {
            let scope = match &self.symbols[symbol].kind {
                SymbolKind::Mod(scope) => *scope,
                _ => return None,
            };
            let name = self.names.id(&segment.ident.name);

            let ns = if segments.peek().is_some() {
                Namespace::Type
            } else {
                namespace
            };
            symbol = self.lookup_in_scope(scope, name, ns)?;
        }

        Some(symbol)
    }

    fn lookup_in_scope(
        &self,
        scope: ScopeId,
        name: NameId,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        let map = match namespace {
            Namespace::Type => &self.scopes[scope].types,
            Namespace::Value => &self.scopes[scope].values,
        };
        map.get(&name).copied()
    }

    fn lookup_up_scope_chain(
        &self,
        mut scope: ScopeId,
        name: NameId,
        namespace: Namespace,
    ) -> Option<SymbolId> {
        loop {
            if let Some(symbol) = self.lookup_in_scope(scope, name, namespace) {
                return Some(symbol);
            }
            scope = self.scopes[scope].parent?;
        }
    }

    fn declare(&mut self, name: &str, span: Span, kind: SymbolKind) -> SymbolId {
        let namespace = kind.namespace();
        let name = self.names.id(name);
        let var = self.uni_cx.fresh_var();
        let ty = term!(self.uni_cx, var var);
        let symbol = self.symbols.insert(Symbol {
            name,
            kind,
            ty,
            generics: Vec::new(),
            declared_at: span,
        });
        self.insert_in_scope(name, symbol, namespace);
        self.positions.record_symbol(span, symbol);
        symbol
    }

    fn declare_generic_param(&mut self, param_name: &str, span: Span) -> (SymbolId, GenericId) {
        let name = self.names.id(param_name);
        let id = self.generic_ids.insert(());
        self.generic_names.declare(id, param_name.to_owned());
        let ty = term!(self.uni_cx, TyCon::Generic(id));
        let symbol = self.symbols.insert(Symbol {
            name,
            kind: SymbolKind::GenericParam,
            ty,
            generics: Vec::new(),
            declared_at: span,
        });
        self.insert_in_scope(name, symbol, Namespace::Type);
        self.positions.record_symbol(span, symbol);
        (symbol, id)
    }

    fn insert_in_scope(&mut self, name: NameId, symbol: SymbolId, namespace: Namespace) {
        let scope = &mut self.scopes[self.current_scope];
        match namespace {
            Namespace::Type => scope.types.insert(name, symbol),
            Namespace::Value => scope.values.insert(name, symbol),
        };
    }

    fn lower_ty(&mut self, ty: &Ty) -> TermId {
        match &ty.kind {
            TyKind::Never => term!(self.uni_cx, TyCon::Never),
            TyKind::Paren(inner) => self.lower_ty(inner),
            TyKind::Array(inner) => term!(self.uni_cx, TyCon::Array => [ self.lower_ty(inner) ]),
            TyKind::Tup(inner) => {
                let args = inner.iter().map(|x| self.lower_ty(x)).collect();
                term!(self.uni_cx, TyCon::Tuple => args)
            }
            TyKind::Fn(fn_ty) => {
                let FnTy { inputs, output } = fn_ty.as_ref();
                let input_args = inputs.iter().map(|x| self.lower_ty(x)).collect();
                let inputs_term = term!(self.uni_cx, TyCon::Tuple => input_args);
                let output_term = match output {
                    FnRetTy::Default(_) => {
                        let var = self.uni_cx.fresh_var();
                        term!(self.uni_cx, var var)
                    }
                    FnRetTy::Ty(ty) => self.lower_ty(ty),
                };
                term!(self.uni_cx, TyCon::Fn => [inputs_term, output_term])
            }
            TyKind::Path(path) => match path.segments.as_slice() {
                [segment] if segment.ident.name == "bool" => {
                    self.positions.record_primitive(segment.ident.span, "bool");
                    term!(self.uni_cx, TyCon::Bool)
                }
                [segment] if segment.ident.name == "int" => {
                    self.positions.record_primitive(segment.ident.span, "int");
                    term!(self.uni_cx, TyCon::Int)
                }
                [segment] if segment.ident.name == "float" => {
                    self.positions.record_primitive(segment.ident.span, "float");
                    term!(self.uni_cx, TyCon::Float)
                }
                [segment] if segment.ident.name == "char" => {
                    self.positions.record_primitive(segment.ident.span, "char");
                    term!(self.uni_cx, TyCon::Char)
                }
                [segment] if segment.ident.name == "String" => {
                    self.positions
                        .record_primitive(segment.ident.span, "String");
                    term!(self.uni_cx, TyCon::Str)
                }
                [segment] if segment.ident.name == "any" => {
                    self.positions.record_primitive(segment.ident.span, "any");
                    term!(self.uni_cx, TyCon::Any)
                }
                _ => match self.resolve_path(path, Namespace::Type) {
                    Some(symbol) => {
                        self.record_path_reference(path, symbol);
                        match &self.symbols[symbol].kind {
                            SymbolKind::Struct => term!(self.uni_cx, TyCon::Struct(symbol)),
                            SymbolKind::Enum => term!(self.uni_cx, TyCon::Enum(symbol)),
                            _ => self.instantiate_path(symbol, path),
                        }
                    }
                    None => term!(self.uni_cx, TyCon::Err),
                },
            },
            TyKind::ImplicitSelf => unimplemented!(),

            TyKind::Infer => {
                let var = self.uni_cx.fresh_var();
                term!(self.uni_cx, var var)
            }
            TyKind::Err => term!(self.uni_cx, TyCon::Err),
        }
    }
}

struct Resolver<'a> {
    cx: &'a mut TypeCheckContext,
}

impl Resolver<'_> {
    fn new_scope(&mut self) -> ScopeId {
        let parent = self.cx.current_scope;
        self.cx.scopes.insert(Scope {
            parent: Some(parent),
            types: HashMap::new(),
            values: HashMap::new(),
        })
    }

    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }
}

impl Visitor for Resolver<'_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => {
                let scope = self.new_scope();
                let fn_symbol = self.cx.declare(
                    &f.ident.name,
                    f.ident.span,
                    SymbolKind::Fn(FnSymbol {
                        scope,
                        param_spans: Vec::new(),
                        param_names: Vec::new(),
                    }),
                );

                let mut generics = Vec::new();
                self.with_scope(scope, |this| {
                    for param in &f.generics.params {
                        let (_, id) = this
                            .cx
                            .declare_generic_param(&param.ident.name, param.ident.span);
                        generics.push(id);
                    }
                    item.walk(this);
                });
                self.cx.symbols[fn_symbol].generics = generics;
                return;
            }
            ItemKind::TyAlias(alias) => {
                let scope = self.new_scope();
                let alias_symbol = self.cx.declare(
                    &alias.ident.name,
                    alias.ident.span,
                    SymbolKind::TyAlias(scope),
                );

                let mut generics = Vec::new();
                self.with_scope(scope, |this| {
                    for param in &alias.generics.params {
                        let (_, id) = this
                            .cx
                            .declare_generic_param(&param.ident.name, param.ident.span);
                        generics.push(id);
                    }
                });
                self.cx.symbols[alias_symbol].generics = generics;
            }
            ItemKind::Enum(ident, _generics, _def) => {
                self.cx.declare(&ident.name, ident.span, SymbolKind::Enum);
            }
            ItemKind::Struct(ident, _generics, data) => {
                let symbol = self.cx.declare(&ident.name, ident.span, SymbolKind::Struct);
                if !matches!(data, VariantData::Struct(_)) {
                    let name = self.cx.names.id(&ident.name);
                    self.cx.insert_in_scope(name, symbol, Namespace::Value);
                }
            }
            ItemKind::Trait(t) => {
                self.cx
                    .declare(&t.ident.name, t.ident.span, SymbolKind::Trait);
            }
            ItemKind::Mod(ident, ModKind::Unloaded) => {
                let scope = self.new_scope();
                self.cx
                    .declare(&ident.name, ident.span, SymbolKind::Mod(scope));
            }
            ItemKind::Mod(ident, ModKind::Loaded(_)) => {
                let scope = self.new_scope();
                self.cx
                    .declare(&ident.name, ident.span, SymbolKind::Mod(scope));
                self.with_scope(scope, |this| item.walk(this));
                return;
            }
            ItemKind::Use(_) | ItemKind::Impl(_) => {}
        }
    }
}

struct SignatureLowerer<'a> {
    cx: &'a mut TypeCheckContext,
}

impl SignatureLowerer<'_> {
    fn with_scope(&mut self, scope: ScopeId, f: impl FnOnce(&mut Self)) {
        let parent = self.cx.current_scope;
        self.cx.current_scope = scope;
        f(self);
        self.cx.current_scope = parent;
    }

    fn lower_fn_sig(&mut self, f: &Fn) -> TermId {
        let inputs = f
            .sig
            .inputs
            .iter()
            .map(|param| match &param.ty {
                Some(ty) => self.cx.lower_ty(ty),
                None => {
                    let var = self.cx.uni_cx.fresh_var();
                    term!(self.cx.uni_cx, var var)
                }
            })
            .collect();
        let inputs_term = term!(self.cx.uni_cx, TyCon::Tuple => inputs);
        let output_term = match &f.sig.output {
            FnRetTy::Default(_) => {
                let var = self.cx.uni_cx.fresh_var();
                term!(self.cx.uni_cx, var var)
            }
            FnRetTy::Ty(ty) => self.cx.lower_ty(ty),
        };
        term!(self.cx.uni_cx, TyCon::Fn => [inputs_term, output_term])
    }
}

impl Visitor for SignatureLowerer<'_> {
    fn visit_item(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Fn(f) => {
                let name = self.cx.names.id(&f.ident.name);
                let symbol = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Value);

                let scope = symbol.and_then(|symbol| match &self.cx.symbols[symbol].kind {
                    SymbolKind::Fn(fn_data) => Some(fn_data.scope),
                    _ => None,
                });
                if let Some(scope) = scope {
                    self.with_scope(scope, |this| {
                        if let Some(symbol) = symbol {
                            let fn_term = this.lower_fn_sig(f);
                            let symbol_ty = this.cx.symbols[symbol].ty;
                            let _ = this.cx.uni_cx.unify(symbol_ty, fn_term);
                            if let SymbolKind::Fn(fn_data) = &mut this.cx.symbols[symbol].kind {
                                fn_data.param_spans = f
                                    .sig
                                    .inputs
                                    .iter()
                                    .map(|p| p.ty.as_ref().map(|ty| ty.span))
                                    .collect();
                                fn_data.param_names = f
                                    .sig
                                    .inputs
                                    .iter()
                                    .map(|p| render::pat_display_name(&p.pat))
                                    .collect();
                            }
                        }
                        item.walk(this);
                    });
                }
            }
            ItemKind::TyAlias(alias) => {
                let name = self.cx.names.id(&alias.ident.name);
                let symbol = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Type);

                let scope = symbol.and_then(|symbol| match &self.cx.symbols[symbol].kind {
                    SymbolKind::TyAlias(scope) => Some(*scope),
                    _ => None,
                });
                if let (Some(symbol), Some(scope), Some(ty)) = (symbol, scope, alias.ty.as_ref()) {
                    self.with_scope(scope, |this| {
                        let aliased = this.cx.lower_ty(ty);
                        let symbol_ty = this.cx.symbols[symbol].ty;
                        let _ = this.cx.uni_cx.unify(symbol_ty, aliased);
                    });
                }
            }
            ItemKind::Mod(ident, ModKind::Loaded(_)) => {
                let name = self.cx.names.id(&ident.name);
                let scope = self
                    .cx
                    .lookup_in_scope(self.cx.current_scope, name, Namespace::Type)
                    .and_then(|symbol| match &self.cx.symbols[symbol].kind {
                        SymbolKind::Mod(scope) => Some(*scope),
                        _ => None,
                    });
                if let Some(scope) = scope {
                    self.with_scope(scope, |this| item.walk(this));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
