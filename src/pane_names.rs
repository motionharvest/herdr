//! Human names for panes.
//!
//! A pane full of agents is hard to scan when every row reads "Pane 1" or
//! repeats the harness name. Each pane gets a stable first name derived from
//! the terminal id. When that name is already taken, the pane takes the next
//! unused name in the list instead, so no two panes share a name. Panes claim
//! names in stable id order, so an existing pane never loses its name when a
//! new one appears. A manual rename still wins over this list, and the name it
//! sets counts as taken.

use std::collections::{HashMap, HashSet};

use crate::terminal::{TerminalId, TerminalState};

const NAMES: &[&str] = &[
    "Abigail",
    "Adam",
    "Alan",
    "Albert",
    "Alex",
    "Alice",
    "Amanda",
    "Amber",
    "Amy",
    "Andrew",
    "Angela",
    "Anna",
    "Anne",
    "Arthur",
    "Ashley",
    "Barbara",
    "Ben",
    "Beth",
    "Betty",
    "Bill",
    "Bob",
    "Brian",
    "Bruce",
    "Carl",
    "Carol",
    "Caroline",
    "Charles",
    "Charlie",
    "Charlotte",
    "Chloe",
    "Chris",
    "Claire",
    "Colin",
    "Craig",
    "Daisy",
    "Dan",
    "Daniel",
    "David",
    "Dennis",
    "Diana",
    "Donald",
    "Donna",
    "Dorothy",
    "Douglas",
    "Edward",
    "Eleanor",
    "Eliza",
    "Ella",
    "Ellen",
    "Emily",
    "Emma",
    "Eric",
    "Ethan",
    "Eve",
    "Frank",
    "Fred",
    "Gary",
    "George",
    "Georgia",
    "Grace",
    "Graham",
    "Hannah",
    "Harold",
    "Harriet",
    "Harry",
    "Heather",
    "Helen",
    "Henry",
    "Holly",
    "Ian",
    "Isabel",
    "Jack",
    "Jacob",
    "Jake",
    "James",
    "Jane",
    "Janet",
    "Jason",
    "Jean",
    "Jeff",
    "Jennifer",
    "Jenny",
    "Jessica",
    "Jill",
    "Jim",
    "Joan",
    "Joe",
    "John",
    "Jonathan",
    "Joseph",
    "Joy",
    "Judy",
    "Julia",
    "Julie",
    "Karen",
    "Kate",
    "Katie",
    "Keith",
    "Kelly",
    "Kevin",
    "Kim",
    "Laura",
    "Lauren",
    "Leo",
    "Lily",
    "Linda",
    "Lisa",
    "Lucy",
    "Luke",
    "Lydia",
    "Margaret",
    "Mark",
    "Martha",
    "Martin",
    "Mary",
    "Matthew",
    "Megan",
    "Michael",
    "Molly",
    "Nancy",
    "Nathan",
    "Neil",
    "Nick",
    "Norman",
    "Oliver",
    "Olivia",
    "Owen",
    "Pamela",
    "Patrick",
    "Paul",
    "Peggy",
    "Peter",
    "Philip",
    "Rachel",
    "Ralph",
    "Rebecca",
    "Richard",
    "Robert",
    "Robin",
    "Roger",
    "Rose",
    "Ruth",
    "Ryan",
    "Sally",
    "Sam",
    "Sarah",
    "Scott",
    "Sean",
    "Simon",
    "Sophie",
    "Stanley",
    "Stephen",
    "Steve",
    "Susan",
    "Thomas",
    "Tim",
    "Tom",
    "Tony",
    "Victoria",
    "Walter",
    "Wendy",
    "William",
    "Zoe",
];

/// Stable base name for a seed string (a terminal id).
#[cfg(test)]
pub fn base_name_for(seed: &str) -> &'static str {
    NAMES[base_index(seed)]
}

/// Assign every terminal without a name of its own a unique name from the
/// list. Each terminal starts at its id's base name and walks forward through
/// the list past names that are already taken, either by an earlier terminal
/// or by a pane someone named. A numeric suffix appears only once every name
/// in the list is in use.
pub fn assigned_names(
    terminals: &HashMap<TerminalId, TerminalState>,
) -> HashMap<TerminalId, String> {
    let mut taken: HashSet<String> = terminals
        .values()
        .filter_map(own_name)
        .map(str::to_ascii_lowercase)
        .collect();

    let mut ids: Vec<&TerminalId> = terminals
        .iter()
        .filter(|(_, terminal)| own_name(terminal).is_none())
        .map(|(id, _)| id)
        .collect();
    ids.sort_by_key(|id| id.to_string());

    let mut names = HashMap::new();
    for id in ids {
        let start = base_index(&id.to_string());
        let name = (0..NAMES.len())
            .map(|offset| NAMES[(start + offset) % NAMES.len()].to_string())
            .chain((2..).map(|round| format!("{}-{round}", NAMES[start])))
            .find(|name| !taken.contains(&name.to_ascii_lowercase()))
            .expect("the suffixed names never run out");
        taken.insert(name.to_ascii_lowercase());
        names.insert(id.clone(), name);
    }
    names
}

/// The name a pane carries independently of the list, if any.
fn own_name(terminal: &TerminalState) -> Option<&str> {
    terminal
        .manual_label
        .as_deref()
        .or(terminal.agent_name.as_deref())
}

fn base_index(seed: &str) -> usize {
    (fnv1a(seed) % NAMES.len() as u64) as usize
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
    fn assigned_names_are_stable_and_unique() {
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
    fn colliding_terminals_take_unused_list_names() {
        let mut terminals = HashMap::new();
        for _ in 0..64 {
            let terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
            terminals.insert(terminal.id.clone(), terminal);
        }
        for name in assigned_names(&terminals).values() {
            assert!(NAMES.contains(&name.as_str()), "name off the list: {name}");
        }
    }

    #[test]
    fn a_manual_name_is_not_assigned_to_another_pane() {
        let mut named = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        let other = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        named.manual_label = Some(base_name_for(&other.id.to_string()).to_ascii_lowercase());
        let mut terminals = HashMap::new();
        terminals.insert(named.id.clone(), named);
        let other_id = other.id.clone();
        terminals.insert(other.id.clone(), other);

        let names = assigned_names(&terminals);
        assert_eq!(names.len(), 1);
        assert_ne!(
            names.get(&other_id).map(String::as_str),
            Some(base_name_for(&other_id.to_string()))
        );
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
        terminals.insert(newcomer.id.clone(), newcomer);
        let after = assigned_names(&terminals);

        for (id, name) in &before {
            assert_eq!(after.get(id), Some(name));
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
