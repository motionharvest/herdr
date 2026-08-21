//! Human names for panes.
//!
//! A pane full of agents is hard to scan when every row reads "Pane 1" or
//! repeats the harness name. Shells and untitled agents get a stable first
//! name derived from the terminal id. Once an agent has a session title, the
//! name is derived from that title. A headline that ends in a past verb uses
//! that verb as an `-er`/`-or` agentive and the last two content nouns before
//! it as the head compound (`Clever Agent Naming Convention Explained` →
//! `naming-convention-explainer`). An imperative title uses the first content
//! verb and the last two content words of its object (`Hide yellow bad weather
//! banner from pages` → `yellow-banner-hider`). A title with no content verb
//! uses `-man` on a one-syllable noun or `-ist` on a longer one. Anything else
//! falls back to the first three words as a slug. That name is frozen on the
//! terminal so a later title refresh does not move the CLI target. Collisions
//! among live terminals are disambiguated with a numeric suffix assigned in
//! stable id order, so an existing pane never loses its name when a new one
//! appears.

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

/// First three words of a session title as a CLI name: lowercase, punctuation
/// dropped, spaces as dashes. `None` when nothing usable remains.
pub fn title_slug(title: &str) -> Option<String> {
    let mut words = Vec::new();
    for raw in title.split_whitespace() {
        let cleaned: String = raw
            .chars()
            .flat_map(char::to_lowercase)
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect();
        if cleaned.is_empty() {
            continue;
        }
        words.push(cleaned);
        if words.len() == 3 {
            break;
        }
    }
    if words.is_empty() {
        None
    } else {
        Some(words.join("-"))
    }
}

const AUXILIARY_LEMMAS: &[&str] = &["be", "have", "do"];

pub fn summary_name(title: &str) -> Option<String> {
    nlp_name(title).or_else(|| title_slug(title))
}

fn nlp_name(title: &str) -> Option<String> {
    let (verb, nouns) = extract_heads(title)?;
    name_from_heads(verb.as_deref(), &nouns)
}

fn name_from_heads(verb: Option<&str>, nouns: &[String]) -> Option<String> {
    if let Some(verb) = verb {
        let verber = kebab(&verber_from_lemma(verb)?);
        if verber.is_empty() {
            return None;
        }
        let noun_parts: Vec<String> = nouns
            .iter()
            .map(|noun| kebab(noun))
            .filter(|noun| !noun.is_empty())
            .collect();
        if noun_parts.is_empty() {
            return Some(verber);
        }
        return Some(format!("{}-{verber}", noun_parts.join("-")));
    }

    let cleaned: Vec<String> = nouns
        .iter()
        .map(|noun| kebab(noun))
        .filter(|noun| !noun.is_empty())
        .collect();
    let head = cleaned.last()?;
    let person = kebab(&person_from_noun(head)?);
    if person.is_empty() {
        return None;
    }
    if cleaned.len() >= 2 {
        let prefix = &cleaned[cleaned.len() - 2];
        if prefix != head && prefix != &person {
            return Some(format!("{prefix}-{person}"));
        }
    }
    Some(person)
}

fn verber_from_lemma(lemma: &str) -> Option<String> {
    let stem = kebab(lemma);
    if stem.is_empty() {
        return None;
    }
    if stem.ends_with("ate") || stem.ends_with("ise") {
        return Some(format!("{}or", &stem[..stem.len() - 1]));
    }
    if stem.ends_with("ct") {
        return Some(format!("{stem}or"));
    }
    if stem.ends_with('e') {
        return Some(format!("{stem}r"));
    }
    if doubles_before_er(&stem) {
        let last = stem.chars().last()?;
        return Some(format!("{stem}{last}er"));
    }
    Some(format!("{stem}er"))
}

fn person_from_noun(noun: &str) -> Option<String> {
    let stem = kebab(noun);
    if stem.is_empty() {
        return None;
    }
    if syllable_count(&stem) <= 1 {
        return Some(format!("{stem}man"));
    }
    let ist_stem = if stem.ends_with('e') || stem.ends_with('y') {
        stem[..stem.len() - 1].to_string()
    } else {
        stem
    };
    if ist_stem.is_empty() {
        return None;
    }
    Some(format!("{ist_stem}ist"))
}

fn kebab(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect()
}

fn doubles_before_er(stem: &str) -> bool {
    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < 3 {
        return false;
    }
    let n = chars.len();
    is_consonant(chars[n - 3])
        && is_vowel(chars[n - 2])
        && is_consonant(chars[n - 1])
        && !matches!(chars[n - 1], 'w' | 'x' | 'y')
}

fn is_vowel(ch: char) -> bool {
    matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')
}

fn is_consonant(ch: char) -> bool {
    ch.is_ascii_alphabetic() && !is_vowel(ch)
}

fn syllable_count(word: &str) -> usize {
    let chars: Vec<char> = word.chars().collect();
    let mut count = 0;
    let mut prev_vowel = false;
    for &ch in &chars {
        let vowel = is_vowel(ch);
        if vowel && !prev_vowel {
            count += 1;
        }
        prev_vowel = vowel;
    }
    if count > 1 && word.ends_with('e') && !word.ends_with("le") {
        count -= 1;
    }
    count.max(1)
}

struct TitleToken {
    word: String,
    clause_end: bool,
}

fn extract_heads(title: &str) -> Option<(Option<String>, Vec<String>)> {
    let tokens = tokenize_title(title);
    if tokens.is_empty() {
        return None;
    }

    if let Some(last) = tokens.last() {
        if let Some(lemma) = past_verb_lemma(&last.word) {
            if !is_auxiliary(&lemma) {
                let words: Vec<String> = tokens[..tokens.len() - 1]
                    .iter()
                    .map(|token| token.word.clone())
                    .collect();
                let nouns = last_content_pair(&words);
                return Some((Some(lemma), nouns));
            }
        }
    }

    let mut verb = None;
    let mut nouns = Vec::new();
    let mut object = Vec::new();
    for token in &tokens {
        if verb.is_none() {
            if is_function_word(&token.word) || is_auxiliary(&token.word) {
                continue;
            }
            if is_base_verb(&token.word) {
                verb = Some(token.word.clone());
                continue;
            }
            if is_adjective(&token.word) {
                continue;
            }
            nouns.push(token.word.clone());
            continue;
        }
        if is_object_cut(&token.word) {
            break;
        }
        object.push(token.word.clone());
        if token.clause_end {
            break;
        }
    }
    if verb.is_some() {
        nouns = object_compound(&object);
    }
    if verb.is_none() && nouns.is_empty() {
        None
    } else {
        Some((verb, nouns))
    }
}

fn tokenize_title(title: &str) -> Vec<TitleToken> {
    title
        .split_whitespace()
        .filter_map(|raw| {
            let word = kebab(raw);
            if word.is_empty() {
                None
            } else {
                let clause_end = raw.chars().any(|ch| matches!(ch, ';' | '.' | '!' | '?'));
                Some(TitleToken { word, clause_end })
            }
        })
        .collect()
}

fn object_compound(tokens: &[String]) -> Vec<String> {
    let mut content = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let token = &tokens[i];
        if is_function_word(token) || is_auxiliary(token) {
            i += 1;
            continue;
        }
        if is_adjective(token) {
            if i + 1 < tokens.len() {
                let next = &tokens[i + 1];
                if !is_function_word(next) && !is_adjective(next) && !is_auxiliary(next) {
                    let later_noun = tokens[i + 2..].iter().any(|rest| {
                        !is_function_word(rest) && !is_adjective(rest) && !is_auxiliary(rest)
                    });
                    if later_noun {
                        i += 2;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }
        content.push(token.clone());
        i += 1;
    }
    last_content_pair(&content)
}

fn last_content_pair(tokens: &[String]) -> Vec<String> {
    let nouns: Vec<String> = tokens
        .iter()
        .filter(|token| !is_function_word(token) && !is_adjective(token) && !is_auxiliary(token))
        .cloned()
        .collect();
    match nouns.len() {
        0 => nouns,
        1 => nouns,
        n => nouns[n - 2..].to_vec(),
    }
}

fn is_object_cut(word: &str) -> bool {
    matches!(
        word,
        "from"
            | "to"
            | "for"
            | "in"
            | "on"
            | "at"
            | "by"
            | "with"
            | "into"
            | "onto"
            | "over"
            | "under"
            | "via"
            | "per"
            | "and"
            | "or"
            | "but"
            | "nor"
            | "if"
            | "when"
            | "while"
            | "than"
            | "then"
            | "as"
            | "vs"
            | "versus"
            | "plus"
    )
}

fn is_function_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "and"
            | "or"
            | "but"
            | "nor"
            | "of"
            | "to"
            | "for"
            | "in"
            | "on"
            | "at"
            | "by"
            | "with"
            | "from"
            | "as"
            | "into"
            | "onto"
            | "over"
            | "under"
            | "than"
            | "then"
            | "vs"
            | "versus"
            | "via"
            | "per"
            | "plus"
            | "not"
            | "no"
            | "if"
            | "when"
            | "while"
            | "that"
            | "this"
            | "these"
            | "those"
            | "it"
            | "its"
    )
}

fn is_adjective(word: &str) -> bool {
    matches!(
        word,
        "clever"
            | "new"
            | "old"
            | "true"
            | "false"
            | "empty"
            | "full"
            | "simple"
            | "easy"
            | "hard"
            | "quick"
            | "slow"
            | "high"
            | "low"
            | "big"
            | "small"
            | "large"
            | "short"
            | "long"
            | "good"
            | "bad"
            | "clean"
            | "dirty"
            | "hidden"
            | "visible"
            | "public"
            | "private"
            | "common"
            | "open"
            | "closed"
            | "ready"
            | "wrong"
            | "right"
            | "first"
            | "last"
            | "next"
            | "current"
            | "latest"
            | "extra"
            | "other"
            | "same"
            | "real"
            | "safe"
            | "local"
            | "remote"
            | "main"
            | "core"
            | "raw"
            | "early"
            | "late"
    ) || word.ends_with("ous")
        || word.ends_with("ful")
        || word.ends_with("less")
        || word.ends_with("ish")
        || word.ends_with("ive")
        || word.ends_with("able")
        || word.ends_with("ible")
        || word.ends_with("ical")
        || word.ends_with("ary")
        || word.ends_with("ory")
}

fn is_base_verb(word: &str) -> bool {
    BASE_VERBS.binary_search(&word).is_ok()
}

fn past_verb_lemma(word: &str) -> Option<String> {
    for (form, lemma) in IRREGULAR_PAST {
        if *form == word {
            return Some((*lemma).to_string());
        }
    }
    strip_ed(word)
}

fn strip_ed(word: &str) -> Option<String> {
    if word.len() < 5 || !word.ends_with("ed") {
        return None;
    }
    let minus_ed = &word[..word.len() - 2];
    let minus_d = &word[..word.len() - 1];
    if minus_ed.ends_with('i') && minus_ed.len() > 1 {
        let mut lemma = minus_ed[..minus_ed.len() - 1].to_string();
        lemma.push('y');
        return Some(lemma);
    }
    let chars: Vec<char> = minus_ed.chars().collect();
    if chars.len() >= 3 {
        let n = chars.len();
        if chars[n - 1] == chars[n - 2] && is_consonant(chars[n - 1]) && is_vowel(chars[n - 3]) {
            return Some(minus_ed[..minus_ed.len() - 1].to_string());
        }
    }
    if minus_d.ends_with("ate")
        || minus_d.ends_with("ise")
        || minus_d.ends_with("ize")
        || minus_d.ends_with("ure")
        || minus_d.ends_with("ive")
        || minus_d.ends_with("ose")
        || minus_d.ends_with("use")
        || minus_d.ends_with("ide")
        || minus_d.ends_with("ade")
        || minus_d.ends_with("ite")
        || minus_d.ends_with("ute")
        || minus_d.ends_with("ete")
        || minus_d.ends_with("age")
        || minus_d.ends_with("ace")
        || minus_d.ends_with("ice")
        || minus_d.ends_with("ame")
        || minus_d.ends_with("ike")
    {
        return Some(minus_d.to_string());
    }
    if minus_ed.len() < 3 {
        return None;
    }
    Some(minus_ed.to_string())
}

fn is_auxiliary(lemma: &str) -> bool {
    AUXILIARY_LEMMAS.contains(&lemma)
}

const IRREGULAR_PAST: &[(&str, &str)] = &[
    ("been", "be"),
    ("broken", "break"),
    ("brought", "bring"),
    ("built", "build"),
    ("caught", "catch"),
    ("chosen", "choose"),
    ("come", "come"),
    ("cut", "cut"),
    ("done", "do"),
    ("drawn", "draw"),
    ("driven", "drive"),
    ("eaten", "eat"),
    ("fallen", "fall"),
    ("felt", "feel"),
    ("found", "find"),
    ("forgotten", "forget"),
    ("given", "give"),
    ("gone", "go"),
    ("got", "get"),
    ("gotten", "get"),
    ("had", "have"),
    ("held", "hold"),
    ("hidden", "hide"),
    ("kept", "keep"),
    ("known", "know"),
    ("left", "leave"),
    ("lost", "lose"),
    ("made", "make"),
    ("meant", "mean"),
    ("met", "meet"),
    ("paid", "pay"),
    ("put", "put"),
    ("read", "read"),
    ("said", "say"),
    ("seen", "see"),
    ("sent", "send"),
    ("set", "set"),
    ("shown", "show"),
    ("sold", "sell"),
    ("spent", "spend"),
    ("split", "split"),
    ("spoken", "speak"),
    ("stood", "stand"),
    ("taken", "take"),
    ("taught", "teach"),
    ("told", "tell"),
    ("thought", "think"),
    ("thrown", "throw"),
    ("understood", "understand"),
    ("won", "win"),
    ("worn", "wear"),
    ("written", "write"),
];

const BASE_VERBS: &[&str] = &[
    "act",
    "add",
    "allow",
    "apply",
    "ask",
    "avoid",
    "build",
    "bump",
    "catch",
    "change",
    "check",
    "clean",
    "clear",
    "close",
    "commit",
    "compare",
    "convert",
    "copy",
    "create",
    "debug",
    "delete",
    "detect",
    "disable",
    "display",
    "drop",
    "enable",
    "expand",
    "export",
    "extract",
    "fetch",
    "fill",
    "find",
    "finish",
    "fix",
    "fold",
    "follow",
    "format",
    "freeze",
    "generate",
    "handle",
    "hide",
    "ignore",
    "implement",
    "import",
    "improve",
    "include",
    "install",
    "keep",
    "land",
    "launch",
    "list",
    "load",
    "make",
    "match",
    "merge",
    "move",
    "open",
    "parse",
    "patch",
    "peek",
    "pick",
    "prevent",
    "prompt",
    "protect",
    "read",
    "reduce",
    "remove",
    "rename",
    "render",
    "replace",
    "report",
    "reset",
    "restore",
    "retry",
    "return",
    "revert",
    "rewrite",
    "run",
    "save",
    "scan",
    "search",
    "send",
    "set",
    "show",
    "skip",
    "sort",
    "split",
    "start",
    "stop",
    "store",
    "strip",
    "switch",
    "sync",
    "test",
    "trim",
    "undo",
    "update",
    "upgrade",
    "use",
    "validate",
    "verify",
    "wait",
    "watch",
    "wrap",
    "write",
];

/// Assign every terminal a unique display name. Terminals whose base names
/// collide are numbered in stable id order ("Olivia", "Olivia-2", …), so
/// earlier terminals keep their bare name when new ones appear. Collision
/// counting is case-insensitive so a word-list `Ada` and a slug `ada` cannot
/// both occupy the same target.
pub fn assigned_names(
    terminals: &HashMap<TerminalId, TerminalState>,
) -> HashMap<TerminalId, String> {
    let mut ids: Vec<&TerminalId> = terminals.keys().collect();
    ids.sort_by_key(|id| id.to_string());

    let mut uses: HashMap<String, usize> = HashMap::new();
    let mut names = HashMap::new();
    for id in ids {
        let Some(terminal) = terminals.get(id) else {
            continue;
        };
        let base = terminal
            .title_name
            .clone()
            .unwrap_or_else(|| base_name_for(&id.to_string()).to_string());
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
        // Force a collision by mapping many terminals into the pool; verify
        // suffix assignment is deterministic and unique.
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
            // A newcomer that collides on base name may renumber its own
            // group depending on id order; unrelated names must never move.
            if base_name_for(&id.to_string()) != newcomer_base {
                assert_eq!(after.get(id), Some(name));
            }
        }
    }

    #[test]
    fn title_slug_takes_the_first_three_words() {
        assert_eq!(
            title_slug("Herdr pane title is Grok generated_title").as_deref(),
            Some("herdr-pane-title")
        );
    }

    #[test]
    fn title_slug_strips_punctuation_and_skips_empty_tokens() {
        assert_eq!(
            title_slug("Hide agent panes; peek instead of replacing").as_deref(),
            Some("hide-agent-panes")
        );
        assert_eq!(
            title_slug("Fix Delete agent + worktree false warning").as_deref(),
            Some("fix-delete-agent")
        );
    }

    #[test]
    fn title_slug_is_none_when_nothing_usable_remains() {
        assert_eq!(title_slug("!!! +++ ---"), None);
        assert_eq!(title_slug("   "), None);
    }

    #[test]
    fn a_frozen_title_name_replaces_the_word_list_name() {
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        let word_list = base_name_for(&terminal.id.to_string()).to_string();
        terminal.title_name = Some("herdr-pane-title".into());
        let mut terminals = HashMap::new();
        terminals.insert(terminal.id.clone(), terminal);
        let names = assigned_names(&terminals);
        let name = names.values().next().unwrap();
        assert_eq!(name, "herdr-pane-title");
        assert_ne!(name, &word_list);
    }

    #[test]
    fn a_later_title_does_not_move_a_frozen_name() {
        let mut terminal = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        terminal.title_name = Some("herdr-pane-title".into());
        terminal.session_title = Some("Something else entirely now".into());
        let mut terminals = HashMap::new();
        terminals.insert(terminal.id.clone(), terminal);
        let names = assigned_names(&terminals);
        assert_eq!(names.values().next().unwrap(), "herdr-pane-title");
    }

    #[test]
    fn colliding_title_names_get_stable_numeric_suffixes() {
        let mut first = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        let mut second = TerminalState::new(TerminalId::alloc(), "/tmp".into());
        first.title_name = Some("fix-the-parser".into());
        second.title_name = Some("fix-the-parser".into());
        let mut terminals = HashMap::new();
        let first_id = first.id.clone();
        let second_id = second.id.clone();
        terminals.insert(first_id.clone(), first);
        terminals.insert(second_id.clone(), second);
        let names = assigned_names(&terminals);
        let mut values: Vec<_> = names.values().cloned().collect();
        values.sort();
        assert_eq!(values, vec!["fix-the-parser", "fix-the-parser-2"]);
        assert_eq!(names, assigned_names(&terminals));
        assert_ne!(names.get(&first_id), names.get(&second_id));
    }

    #[test]
    fn verber_uses_er_or_and_doubling() {
        assert_eq!(verber_from_lemma("hide").as_deref(), Some("hider"));
        assert_eq!(verber_from_lemma("commit").as_deref(), Some("committer"));
        assert_eq!(verber_from_lemma("create").as_deref(), Some("creator"));
        assert_eq!(verber_from_lemma("act").as_deref(), Some("actor"));
    }

    #[test]
    fn person_from_noun_uses_man_or_ist() {
        assert_eq!(person_from_noun("work").as_deref(), Some("workman"));
        assert_eq!(person_from_noun("craft").as_deref(), Some("craftman"));
        assert_eq!(person_from_noun("machine").as_deref(), Some("machinist"));
        assert_eq!(person_from_noun("art").as_deref(), Some("artman"));
        assert_eq!(person_from_noun("theory").as_deref(), Some("theorist"));
    }

    #[test]
    fn name_from_heads_pairs_noun_and_verber() {
        assert_eq!(
            name_from_heads(Some("commit"), &["work".into()]).as_deref(),
            Some("work-committer")
        );
        assert_eq!(
            name_from_heads(Some("hide"), &["agent".into()]).as_deref(),
            Some("agent-hider")
        );
        assert_eq!(
            name_from_heads(Some("land"), &[]).as_deref(),
            Some("lander")
        );
        assert_eq!(
            name_from_heads(Some("explain"), &["naming".into(), "convention".into()]).as_deref(),
            Some("naming-convention-explainer")
        );
        assert_eq!(
            name_from_heads(None, &["pane".into(), "title".into()]).as_deref(),
            Some("pane-titlist")
        );
        assert_eq!(
            name_from_heads(None, &["work".into()]).as_deref(),
            Some("workman")
        );
        assert_eq!(
            name_from_heads(None, &["machine".into()]).as_deref(),
            Some("machinist")
        );
    }

    #[test]
    fn summary_name_uses_nlp_then_slug() {
        assert_eq!(
            summary_name("Commit work and land herdr worktree").as_deref(),
            Some("work-committer")
        );
        assert_eq!(summary_name("!!! +++ ---"), None);
        assert!(summary_name("Herdr pane title is Grok generated_title").is_some());
    }

    #[test]
    fn a_headline_title_uses_the_final_verb_and_head_compound() {
        assert_eq!(
            summary_name("Clever Agent Naming Convention Explained").as_deref(),
            Some("naming-convention-explainer")
        );
    }

    #[test]
    fn an_imperative_title_still_uses_the_first_verb_and_object() {
        assert_eq!(
            summary_name("Hide agent panes; peek instead of replacing").as_deref(),
            Some("agent-panes-hider")
        );
        assert_eq!(
            summary_name("Fix the parser").as_deref(),
            Some("parser-fixer")
        );
    }

    #[test]
    fn an_imperative_title_uses_a_three_segment_object_compound() {
        assert_eq!(
            summary_name("Hide yellow bad weather banner from pages").as_deref(),
            Some("yellow-banner-hider")
        );
        assert_eq!(
            summary_name("Add Playbill link to main navigation").as_deref(),
            Some("playbill-link-adder")
        );
        assert_eq!(
            summary_name("Add Donate Auction Survey nav links").as_deref(),
            Some("nav-links-adder")
        );
    }

    #[test]
    fn past_verb_lemma_strips_regular_and_irregular_forms() {
        assert_eq!(past_verb_lemma("explained").as_deref(), Some("explain"));
        assert_eq!(past_verb_lemma("created").as_deref(), Some("create"));
        assert_eq!(past_verb_lemma("committed").as_deref(), Some("commit"));
        assert_eq!(past_verb_lemma("shown").as_deref(), Some("show"));
        assert_eq!(past_verb_lemma("naming"), None);
    }

    #[test]
    fn base_verbs_are_sorted_for_binary_search() {
        let mut sorted = BASE_VERBS.to_vec();
        sorted.sort_unstable();
        assert_eq!(BASE_VERBS, sorted.as_slice());
    }
}
