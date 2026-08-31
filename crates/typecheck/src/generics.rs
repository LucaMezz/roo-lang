//! Contains the [`GenericRegistry`] struct, the registry of every generic
//! type parameter that exists in the program: its identity and its
//! display name. Also contains [`SyntheticNames`], a local, one-off
//! source of fresh generic parameter names for a single generalisation
//! or synthesis scope.
use std::collections::HashSet;

#[derive(Default)]
pub(crate) struct SyntheticNames {
    /// Names that must not be handed out: reserved via
    /// [`Self::reserve`], or returned by an earlier call to
    /// [`Self::fresh`] on this same instance.
    taken: HashSet<String>,

    /// How many names this scope has synthesised so far.
    counter: u32,
}

impl SyntheticNames {
    /// Starts a new, empty source of fresh names for one scope.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Reserves `name` so this scope's synthesis skips it. Used for
    /// names local to this scope that aren't yet in the registry --
    /// e.g. a function's own explicit generic type parameters, which
    /// still need to be avoided even though the function that owns
    /// them may not itself have been generalised yet. For example:
    /// ```ignore
    /// fn pair<T>(x: T, y) {
    ///     (x, y)
    /// }
    /// ```
    /// The function above has an explicit generic type parameter `T`,
    /// but the variable `y` has no bound type. Reserving `T` before
    /// synthesising means the next call to [`Self::fresh`] returns
    /// `U`, not `T`, giving the function signature:
    /// ```ignore
    /// fn pair<T, U>(x: T, y: U) -> (T, U)
    /// ```
    pub(crate) fn reserve(&mut self, name: String) {
        self.taken.insert(name);
    }

    /// Generates a new synthetic name for a generic type parameter
    /// based on how many names this scope has synthesised so far.
    fn next(&mut self) -> String {
        const LETTERS: [char; 7] = ['T', 'U', 'V', 'W', 'X', 'Y', 'Z'];
        let n = self.counter;
        self.counter += 1;
        let letter = LETTERS[(n % LETTERS.len() as u32) as usize];
        let suffix = n / LETTERS.len() as u32;
        if suffix == 0 {
            letter.to_string()
        } else {
            format!("{letter}{}", suffix + 1)
        }
    }

    /// Returns the next name in this scope that isn't already taken.
    pub(crate) fn fresh(&mut self) -> String {
        loop {
            let name = self.next();
            if self.taken.insert(name.clone()) {
                return name;
            }
        }
    }
}
