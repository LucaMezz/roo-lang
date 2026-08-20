//! Contains definition of the [`Diagnostic`] struct that
//! represents a diagnostic which can be consumed by
//! other crates, which contains the rendered messages.
//!
//! Crates can define a catalog of diagnostics which they
//! may emit. A .ftl file for each locale can then be
//! created which contains the written messages associated
//! with each diagnostic.
//!
//! This module also facilitates rendering diagnostics,
//! where the actual messages are separated into different
//! `Fluent` .ftl files. This is so that the messages
//! can be localised.
use std::ops::Range;

use ast::Span;
use fluent_bundle::{FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle};
use unic_langid::LanguageIdentifier;

/// The severity of a diagnostic. Errors should cause a script
/// to fail running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// A fatal error. This prevents the script from being
    /// interpreted.
    Error,
    /// A warning which is not fatal, but points to a
    /// potential issue.
    Warning,
    /// A general note.
    Note,
    /// A helpful message.
    Help,
}

/// A unique error code to identify a particular diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ErrorCode(pub u32);

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "E{:04}", self.0)
    }
}

/// A diagnostic produced by the compiler to report an error,
/// warning, or other issue in the source code.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// The span within the source code that resulted in this
    /// diagnostic being raised.
    primary_span: Span,
    /// The severity of the diagnostic.
    level: Level,
    /// The main message describing the reason this diagnostic
    /// was produced.
    message: String,
    /// Contains zero or more spans within the source code that
    /// are directly related to this diagnostic, each including
    /// a message describing that relationship.
    related: Vec<(Span, String)>,
    /// Extra information about the diagnostic.
    notes: Vec<String>,
    /// Areas which should be emphasised within the diagnostic.
    emphasis: Vec<Range<usize>>,
}

impl Diagnostic {
    /// Returns the severity of the diagnostic.
    pub fn level(&self) -> Level {
        self.level
    }

    /// Returns the main message describing the reason for the
    /// diagnostic.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the main span within the source code which
    /// caused the diagnostic to be emitted.
    pub fn span(&self) -> Span {
        self.primary_span
    }

    /// A number of tuples, each representing a piece of
    /// related information about the diagnostic.
    ///
    /// Each includes the span in the source where this related
    /// piece of information exists, as well as a message
    /// describing that relationship.
    pub fn related(&self) -> &[(Span, String)] {
        &self.related
    }

    /// Returns the notes about this diagnostic.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Returns the areas of the main message which should be
    /// embpasised, e.g. highlighted.
    pub fn emphasis(&self) -> &[Range<usize>] {
        &self.emphasis
    }
}

/// A small closed set of value types that can be interpolated
/// into Fluent messages.
#[derive(Debug, Clone)]
pub enum ArgValue {
    /// A number
    Number(i64),
    /// Text
    Text(String),
}

/// A type which can be converted to an [`ArgValue`].
/// This allows it to be interpolated into Fluent messages.
pub trait ToArgValue {
    /// Converts to an [`ArgValue`].
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

/// Information related to a main diagnostic.
#[derive(Debug, Clone)]
pub struct Related {
    /// The span this related info points to.
    pub span: Span,
    /// The Fluent message id to render.
    pub message_id: &'static str,
    /// Arguments interpolated into the message.
    pub args: Vec<(&'static str, ArgValue)>,
}

/// An extra note attached to a diagnostic.
#[derive(Debug, Clone)]
pub struct Note {
    /// The Fluent message id to render.
    pub message_id: &'static str,
    /// Arguments interpolated into the message.
    pub args: Vec<(&'static str, ArgValue)>,
}

/// A single kind of diagnostic a crate can emit, before rendering.
pub trait Diagnose {
    /// This diagnostic's unique error code.
    const CODE: ErrorCode;
    /// This diagnostic's severity.
    const LEVEL: Level;

    /// The primary span the diagnostic is raised at.
    fn span(&self) -> Span;
    /// The Fluent message id for the main message.
    fn message_id(&self) -> &'static str;
    /// Arguments interpolated into the main message.
    fn args(&self) -> Vec<(&'static str, ArgValue)>;

    /// Pairs of (container arg, fragment arg) to highlight within
    /// the rendered message.
    fn emphasize(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Other spans related to this diagnostic.
    fn related(&self) -> Vec<Related> {
        Vec::new()
    }

    /// Extra notes to attach to this diagnostic.
    fn notes(&self) -> Vec<Note> {
        Vec::new()
    }
}

/// Renders [`Diagnose`] values into [`Diagnostic`]s for a given locale.
pub struct Catalog {
    bundle: FluentBundle<FluentResource>,
}

impl Catalog {
    /// Builds a catalog for `locale` from Fluent `.ftl` sources.
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

    /// Renders a [`Diagnose`] value's messages and computes its
    /// emphasis ranges, producing a [`Diagnostic`].
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

/// Declares an enum of [`Diagnose`] types a crate can emit, along
/// with a `render` method and a test asserting error codes are unique.
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
