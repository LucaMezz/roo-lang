use std::ops::Range;

use ast::{Pat, PatKind, Span};
use slotmap::SlotMap;
use unify::{Term, UnificationContext};

use crate::generic_names::GenericNames;
#[cfg(test)]
use crate::{FnSymbol, GenericId, SymbolKind};
use crate::{NameInterner, Symbol, SymbolId, TermId, TyCon, TypeCheckContext};

pub(crate) fn pat_display_name(pat: &Pat) -> String {
    match &pat.kind {
        PatKind::Ident(ident, _) => ident.name.clone(),
        _ => "_".to_owned(),
    }
}

#[cfg(test)]
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

pub(crate) struct Renderer<'a> {
    uni_cx: &'a mut UnificationContext<TyCon, Span>,
    symbols: &'a SlotMap<SymbolId, Symbol>,
    names: &'a NameInterner,
    generic_names: &'a GenericNames,
}

impl TypeCheckContext {
    pub(crate) fn renderer(&mut self) -> Renderer<'_> {
        Renderer {
            uni_cx: &mut self.uni_cx,
            symbols: &self.symbols,
            names: &self.names,
            generic_names: &self.generic_names,
        }
    }

    #[cfg(test)]
    pub(crate) fn render_symbol_type(&mut self, symbol: SymbolId) -> String {
        self.renderer().render_symbol_type(symbol)
    }

    #[cfg(test)]
    pub(crate) fn describe_symbol(&mut self, symbol: SymbolId, at: usize) -> String {
        self.renderer().describe_symbol(symbol, at)
    }
}

impl Renderer<'_> {
    pub(crate) fn render_term(&mut self, term: TermId) -> String {
        let mut buf = String::new();
        self.render_term_into(&mut buf, term, None);
        buf
    }

    pub(crate) fn render_term_highlighting(
        &mut self,
        term: TermId,
        highlight: TermId,
    ) -> (String, Option<Range<usize>>) {
        let mut buf = String::new();
        let range = self.render_term_into(&mut buf, term, Some(highlight));
        (buf, range)
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

    #[cfg(test)]
    pub(crate) fn render_symbol_type(&mut self, symbol: SymbolId) -> String {
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

    #[cfg(test)]
    pub(crate) fn describe_symbol(&mut self, symbol: SymbolId, at: usize) -> String {
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

    #[cfg(test)]
    fn symbol_display_name(&mut self, symbol: SymbolId) -> String {
        let name = self.symbols[symbol].name;
        self.names
            .name(name)
            .cloned()
            .unwrap_or_else(|| "_".to_owned())
    }

    #[cfg(test)]
    fn alias_name_with_generics(&mut self, symbol: SymbolId) -> String {
        let name = self.symbol_display_name(symbol);
        let generics = self.symbols[symbol].generics.clone();
        let generics_rendered = generics_list(self.generic_names, &generics);
        format!("{name}{generics_rendered}")
    }

    #[cfg(test)]
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
