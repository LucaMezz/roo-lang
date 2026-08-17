use std::collections::{HashMap, HashSet};

use crate::GenericId;

#[derive(Default)]
pub(crate) struct GenericNames {
    names: HashMap<GenericId, String>,
    synthetic_counter: u32,
}

impl GenericNames {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn declare(&mut self, id: GenericId, name: String) {
        self.names.insert(id, name);
    }

    pub(crate) fn get(&self, id: &GenericId) -> Option<&String> {
        self.names.get(id)
    }

    pub(crate) fn reset_synthetic_counter(&mut self) {
        self.synthetic_counter = 0;
    }

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

    pub(crate) fn fresh_synthetic(&mut self, taken: &mut HashSet<String>) -> String {
        loop {
            let name = self.next_synthetic();
            if taken.insert(name.clone()) {
                return name;
            }
        }
    }
}
