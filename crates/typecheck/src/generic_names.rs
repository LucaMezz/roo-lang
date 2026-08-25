//! Contains the [`GenericNames`] struct, with methods that
//! facilitate the naming and storage of generic type parameters
//! throughout the type checking process.
use std::collections::{HashMap, HashSet};

use crate::GenericId;

/// Facilitates the storage and synthesis of names for generic
/// parameters throughout the type checking process.
///
/// Synthesis of a generic parameter name occurs whenever a
/// function or a group of functions generalise a free type
/// inference variable into a new generic type parameter.
/// These synthesised generic parameters are assigned names
/// so that these parameters can be displayed as any other
/// generic parameter would be in diagnostics, etc.
#[derive(Default)]
pub(crate) struct GenericNames {
    /// A mapping from [`GenericId`] to the name of the
    /// generic parameter.
    names: HashMap<GenericId, String>,

    /// A counter that tracks how many generic parameters
    /// have already been synthesised within the current
    /// context.
    synthetic_counter: u32,
}

impl GenericNames {
    /// Creates a new blank [`GenericNames`].
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Stores a new name for the given generic parameter.
    pub(crate) fn declare(&mut self, id: GenericId, name: String) {
        self.names.insert(id, name);
    }

    /// Retireves the name of a generic parameter, if it has
    /// one.
    pub(crate) fn get(&self, id: &GenericId) -> Option<&String> {
        self.names.get(id)
    }

    /// Returns every name assigned to a generic parameter so far,
    /// whether explicit or synthesised. Used to seed the `taken`
    /// set before generalising a new function or group of
    /// functions, so that a synthesised name never collides with
    /// one already assigned to a *different* [`GenericId`] that
    /// may end up embedded in the same rendered signature (e.g. a
    /// nested function's generic showing up inside its enclosing
    /// function's type once the enclosing function returns it).
    pub(crate) fn all_names(&self) -> HashSet<String> {
        self.names.values().cloned().collect()
    }

    /// Resets the internal synthetic counter back to 0. Used
    /// whenever beginning to generalise a new function or group
    /// of functions, since we would like them to reuse the same
    /// generic names.
    pub(crate) fn reset_synthetic_counter(&mut self) {
        self.synthetic_counter = 0;
    }

    /// Generates a new synthetic name for a generic type parameter
    /// based on the number of generic names already synthesised.
    /// For example:
    /// ```ignore
    /// fn identity(x) {
    ///     x
    /// }
    /// ```
    /// `x` has no bound type, so generalisation introduces a new
    /// generic type parameter. `next_synthetic` is called to
    /// assign a name to such a parameter, and so the inferred
    /// signature of the function would be
    /// ```ignore
    /// fn identity(x: T) -> T
    /// ```
    fn next_synthetic(&mut self) -> String {
        const LETTERS: [char; 7] = ['T', 'U', 'V', 'W', 'X', 'Y', 'Z'];
        let n = self.synthetic_counter;
        self.synthetic_counter += 1;
        let letter = LETTERS[(n % LETTERS.len() as u32) as usize];
        let suffix = n / LETTERS.len() as u32;
        if suffix == 0 {
            letter.to_string()
        } else {
            format!("{letter}{}", suffix + 1)
        }
    }

    /// Generates a new synthetic name for a generic type parameter
    /// based on the number of generic names already synthesised, and
    /// given a set of names which have already been taken.
    ///
    /// Used in functions which may already have explicit generic
    /// type parameters included in the source, but which still has
    /// free inference variables that need generalisation. This is
    /// used to prevent a synthetic name from colliding with the
    /// existing explicit names. For example:
    /// ```ignore
    /// fn pair<T>(x: T, y) {
    ///     (x, y)
    /// }
    /// ```
    /// The function above has an explicit generic type parameter `T`,
    /// but the variable `y` has no bound type. Generalisation
    /// calls `fresh_synthetic` with `T` in the taken set, which will
    /// ignore the `T` and instead return `U` which is not taken.
    /// This infers the function signature to be:
    /// ```ignore
    /// fn pair<T, U>(x: T, y: U) -> (T, U)
    /// ```
    pub(crate) fn fresh_synthetic(&mut self, taken: &mut HashSet<String>) -> String {
        std::iter::from_fn(|| Some(self.next_synthetic()))
            .find(|name| taken.insert(name.clone()))
            .expect("infinitely many candidate names are generated")
    }
}
