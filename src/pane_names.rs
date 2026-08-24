//! Human names for panes.
//!
//! A pane full of agents is hard to scan when every row reads "Pane 1" or
//! repeats the harness name. Each pane gets a stable first name derived from
//! the terminal id. Collisions among live terminals are disambiguated with a
//! numeric suffix assigned in stable id order, so an existing pane never loses
//! its name when a new one appears. A manual rename still wins over this list.

use std::collections::HashMap;

use crate::terminal::{TerminalId, TerminalState};

const NAMES: &[&str] = &[
    "Ada", "Aiden", "Alice", "Amara", "Amelia", "Amir", "Anders", "Anika", "Anton", "Aria",
    "Arjun", "Asha", "Astrid", "Aurora", "Axel", "Bailey", "Beatrix", "Bella", "Benji", "Bianca",
    "Bodhi", "Boris", "Bruno", "Callie", "Camila", "Carmen", "Caspian", "Cecilia", "Chidi",
    "Clara", "Cleo", "Cole", "Cyrus", "Dahlia", "Dante", "Daphne", "Darius", "Delia", "Dexter",
    "Diego", "Dinah", "Dmitri", "Eamon", "Edith", "Elena", "Elias", "Elsa", "Emeka", "Emil",
    "Esme", "Ezra", "Farah", "Felix", "Fern", "Finn", "Flora", "Freya", "Gideon", "Greta",
    "Gustav", "Hana", "Harvey", "Hazel", "Hector", "Hugo", "Ibrahim", "Ida", "Igor", "Iker",
    "Imani", "Indira", "Ingrid", "Irene", "Isaac", "Isla", "Ivan", "Ivy", "Jasper", "Jorge",
    "Juno", "Kai", "Kamal", "Kara", "Keiko", "Kenji", "Kiara", "Kirby", "Lars", "Layla", "Leif",
    "Lena", "Leo", "Lila", "Linnea", "Lorenzo", "Lucia", "Luka", "Luna", "Maeve", "Magnus",
    "Malik", "Mara", "Marco", "Margot", "Mateo", "Maya", "Mei", "Micah", "Milo", "Mina", "Mira",
    "Moira", "Nadia", "Naomi", "Nash", "Nia", "Nico", "Nikolai", "Nina", "Noor", "Nova", "Oberon",
    "Odessa", "Olga", "Olivia", "Omar", "Onyx", "Oscar", "Otis", "Otto", "Paloma", "Paolo",
    "Pearl", "Petra", "Phoebe", "Pia", "Piper", "Priya", "Quentin", "Quinn", "Rafael", "Ravi",
    "Remy", "Renata", "Rhea", "Rocco", "Rosa", "Rowan", "Ruby", "Rufus", "Sable", "Sage", "Saki",
    "Salma", "Sanjay", "Sasha", "Selene", "Silas", "Simone", "Sofia", "Soren", "Stella", "Suki",
    "Sven", "Tara", "Tessa", "Thea", "Theo", "Tilda", "Tobias", "Tova", "Uma", "Uri", "Ursula",
    "Vera", "Vidal", "Viggo", "Vikram", "Viola", "Wanda", "Wendell", "Willa", "Xander", "Xenia",
    "Yara", "Yusuf", "Yuki", "Zadie", "Zane", "Zara", "Zelda", "Ziggy", "Zoe", "Zora",
];

/// Stable base name for a seed string (a terminal id).
pub fn base_name_for(seed: &str) -> &'static str {
    NAMES[(fnv1a(seed) % NAMES.len() as u64) as usize]
}

/// Assign every terminal a unique display name. Terminals whose base names
/// collide are numbered in stable id order ("Olivia", "Olivia-2", …), so
/// earlier terminals keep their bare name when new ones appear.
pub fn assigned_names(
    terminals: &HashMap<TerminalId, TerminalState>,
) -> HashMap<TerminalId, String> {
    let mut ids: Vec<&TerminalId> = terminals.keys().collect();
    ids.sort_by_key(|id| id.to_string());

    let mut uses: HashMap<String, usize> = HashMap::new();
    let mut names = HashMap::new();
    for id in ids {
        let base = base_name_for(&id.to_string()).to_string();
        let key = base.to_ascii_lowercase();
        let count = uses.entry(key).or_insert(0);
        *count += 1;
        let name = if *count == 1 {
            base
        } else {
            format!("{base}-{count}")
        };
        names.insert(id.clone(), name);
    }
    names
}

fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_names_are_stable_for_a_seed() {
        assert_eq!(base_name_for("term_abc"), base_name_for("term_abc"));
    }

    #[test]
    fn name_pool_entries_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for name in NAMES {
            assert!(seen.insert(*name), "duplicate pool name: {name}");
        }
    }

    #[test]
    fn colliding_terminals_get_stable_numeric_suffixes() {
        let mut terminals = HashMap::new();
        for _ in 0..64 {
            let terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
            terminals.insert(terminal.id.clone(), terminal);
        }

        let first = assigned_names(&terminals);
        let second = assigned_names(&terminals);
        assert_eq!(first, second);

        let mut seen = std::collections::HashSet::new();
        for name in first.values() {
            assert!(seen.insert(name.clone()), "duplicate assigned name: {name}");
        }
    }

    #[test]
    fn existing_names_survive_new_terminals() {
        let mut terminals = HashMap::new();
        for _ in 0..8 {
            let terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
            terminals.insert(terminal.id.clone(), terminal);
        }
        let before = assigned_names(&terminals);

        let newcomer = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        let newcomer_base = base_name_for(&newcomer.id.to_string());
        terminals.insert(newcomer.id.clone(), newcomer);
        let after = assigned_names(&terminals);

        for (id, name) in &before {
            if base_name_for(&id.to_string()) != newcomer_base {
                assert_eq!(after.get(id), Some(name));
            }
        }
    }

    #[test]
    fn a_stored_title_slug_does_not_replace_the_word_list_name() {
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        let word_list = base_name_for(&terminal.id.to_string()).to_string();
        terminal.title_name = Some("herdr-pane-title".into());
        let mut terminals = HashMap::new();
        terminals.insert(terminal.id.clone(), terminal);
        let names = assigned_names(&terminals);
        assert_eq!(names.values().next().unwrap(), &word_list);
    }
}
