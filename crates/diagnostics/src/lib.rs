use std::ops::Range;

use ast::Span;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle};
use unic_langid::LanguageIdentifier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub u32);

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "E{:04}", self.0)
    }
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    primary_span: Span,
    level: Level,
    message: String,
    related: Vec<(Span, String)>,
    notes: Vec<String>,
    emphasis: Vec<Range<usize>>,
}

impl Diagnostic {
    pub fn level(&self) -> Level {
        self.level
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn span(&self) -> Span {
        self.primary_span
    }

    pub fn related(&self) -> &[(Span, String)] {
        &self.related
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    pub fn emphasis(&self) -> &[Range<usize>] {
        &self.emphasis
    }
}

#[derive(Debug, Clone)]
pub enum ArgValue {
    Number(i64),
    Text(String),
}

pub trait ToArgValue {
    fn to_arg_value(&self) -> ArgValue;
}

impl ToArgValue for usize {
    fn to_arg_value(&self) -> ArgValue {
        ArgValue::Number(*self as i64)
    }
}

impl ToArgValue for i64 {
    fn to_arg_value(&self) -> ArgValue {
        ArgValue::Number(*self)
    }
}

impl ToArgValue for String {
    fn to_arg_value(&self) -> ArgValue {
        ArgValue::Text(self.clone())
    }
}

impl ToArgValue for str {
    fn to_arg_value(&self) -> ArgValue {
        ArgValue::Text(self.to_owned())
    }
}

#[derive(Debug, Clone)]
pub struct Related {
    pub span: Span,
    pub message_id: &'static str,
    pub args: Vec<(&'static str, ArgValue)>,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub message_id: &'static str,
    pub args: Vec<(&'static str, ArgValue)>,
}

pub trait Diagnose {
    const CODE: ErrorCode;
    const LEVEL: Level;

    fn span(&self) -> Span;
    fn message_id(&self) -> &'static str;
    fn args(&self) -> Vec<(&'static str, ArgValue)>;

    fn emphasize(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    fn related(&self) -> Vec<Related> {
        Vec::new()
    }

    fn notes(&self) -> Vec<Note> {
        Vec::new()
    }
}

pub struct Catalog {
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    pub fn new(locale: &str, sources: &[&str]) -> Self {
        let language: LanguageIdentifier = locale.parse().expect("invalid locale identifier");
        let mut bundle = FluentBundle::new_concurrent(vec![language]);
        bundle.set_use_isolating(false);
        for source in sources {
            let resource =
                FluentResource::try_new(source.to_string()).expect("invalid fluent resource");
            bundle
                .add_resource(resource)
                .expect("duplicate fluent message id");
        }
        Self { bundle }
    }

    pub fn render<D: Diagnose>(&self, diagnostic: &D) -> Diagnostic {
        let args = diagnostic.args();
        let message = self.format(diagnostic.message_id(), &args);

        let mut claimed: Vec<Range<usize>> = Vec::new();
        let mut emphasis = Vec::new();
        for (container, fragment) in diagnostic.emphasize() {
            let Some(container_text) = text_of(&args, container) else {
                continue;
            };
            let Some(fragment_text) = text_of(&args, fragment) else {
                continue;
            };
            if let Some(range) = locate(&message, container_text, fragment_text, &claimed) {
                claimed.push(range.clone());
                emphasis.push(range);
            }
        }

        let related = diagnostic
            .related()
            .into_iter()
            .map(|related| (related.span, self.format(related.message_id, &related.args)))
            .collect();
        let notes = diagnostic
            .notes()
            .into_iter()
            .map(|note| self.format(note.message_id, &note.args))
            .collect();

        Diagnostic {
            primary_span: diagnostic.span(),
            level: D::LEVEL,
            message,
            related,
            notes,
            emphasis,
        }
    }

    fn format(&self, id: &str, args: &[(&'static str, ArgValue)]) -> String {
        let fluent_args = to_fluent_args(args);
        let message = self
            .bundle
            .get_message(id)
            .unwrap_or_else(|| panic!("missing fluent message: {id}"));
        let pattern = message
            .value()
            .unwrap_or_else(|| panic!("fluent message has no value: {id}"));
        let mut errors = Vec::new();
        let formatted = self
            .bundle
            .format_pattern(pattern, Some(&fluent_args), &mut errors);
        assert!(errors.is_empty(), "fluent formatting errors: {errors:?}");
        formatted.into_owned()
    }
}

fn to_fluent_args(args: &[(&'static str, ArgValue)]) -> FluentArgs<'static> {
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
        let value = match value {
            ArgValue::Number(n) => FluentValue::from(*n),
            ArgValue::Text(s) => FluentValue::from(s.clone()),
        };
        fluent_args.set(*name, value);
    }
    fluent_args
}

fn text_of<'a>(args: &'a [(&'static str, ArgValue)], name: &str) -> Option<&'a str> {
    args.iter()
        .find(|(n, _)| *n == name)
        .and_then(|(_, v)| match v {
            ArgValue::Text(s) => Some(s.as_str()),
            ArgValue::Number(_) => None,
        })
}

fn overlaps(claimed: &[Range<usize>], range: &Range<usize>) -> bool {
    claimed
        .iter()
        .any(|c| c.start < range.end && range.start < c.end)
}

fn locate(
    message: &str,
    container_text: &str,
    fragment_text: &str,
    claimed: &[Range<usize>],
) -> Option<Range<usize>> {
    let mut search_from = 0;
    while let Some(rel_start) = message[search_from..].find(container_text) {
        let container_start = search_from + rel_start;
        let container_end = container_start + container_text.len();
        if !overlaps(claimed, &(container_start..container_end))
            && let Some(frag_rel) = container_text.find(fragment_text)
        {
            let start = container_start + frag_rel;
            let end = start + fragment_text.len();
            let range = start..end;
            if !overlaps(claimed, &range) {
                return Some(range);
            }
        }
        search_from = container_start + 1;
    }
    None
}

#[macro_export]
macro_rules! catalog {
    ($vis:vis enum $enum_name:ident { $($name:ident),+ $(,)? }) => {
        $vis enum $enum_name {
            $($name($name)),+
        }

        $(
            impl ::std::convert::From<$name> for $enum_name {
                fn from(value: $name) -> Self {
                    $enum_name::$name(value)
                }
            }
        )+

        impl $enum_name {
            pub fn render(&self, catalog: &$crate::Catalog) -> $crate::Diagnostic {
                match self {
                    $($enum_name::$name(inner) => catalog.render(inner)),+
                }
            }
        }

        #[cfg(test)]
        mod __catalog_tests {
            use super::*;

            #[test]
            fn error_codes_are_unique() {
                let codes: &[u32] = &[$(<$name as $crate::Diagnose>::CODE.0),+];
                let mut seen = ::std::collections::HashSet::new();
                for &code in codes {
                    assert!(seen.insert(code), "duplicate error code E{code:04}");
                }
            }
        }
    };
}
