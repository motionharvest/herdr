//! A text field that holds one or more lines.
//!
//! Both the folder-path field and the task field are this type. The path field
//! simply never receives a newline, so there is no single-line variant to keep
//! in step with this one.
//!
//! Positions are counted in characters, not bytes, so a multi-byte character
//! moves the cursor one step like any other.

/// One row of the field as it is drawn: which line it came from, where in that
/// line it starts, and the text on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub line: usize,
    pub start: usize,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct TextField {
    lines: Vec<String>,
    line: usize,
    col: usize,
    /// How wide the field is drawn, which is what `up` and `down` need in order
    /// to step by rows on screen rather than by lines in the text. Zero means
    /// the field is not wrapped and the two are the same thing.
    width: usize,
}

impl Default for TextField {
    fn default() -> Self {
        Self {
            lines: vec![String::new()],
            line: 0,
            col: 0,
            width: 0,
        }
    }
}

impl TextField {
    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
    }

    /// Whether the cursor sits after the last character, which is where a
    /// completion can be drawn without covering letters already in the field.
    pub fn at_end(&self) -> bool {
        self.line + 1 == self.lines.len() && self.col == self.len(self.line)
    }

    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(str::to_string).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.line = self.lines.len() - 1;
        self.col = self.len(self.line);
    }

    pub fn clear(&mut self) {
        self.lines = vec![String::new()];
        self.line = 0;
        self.col = 0;
    }

    /// Tell the field how wide it is drawn. Everything else about it is
    /// unchanged; only `up`, `down`, and the rows it reports depend on this.
    pub fn set_width(&mut self, width: usize) {
        self.width = width;
    }

    /// The field broken into the rows it is drawn on, wrapping at spaces. A word
    /// longer than the field is broken where it runs out of room, since the
    /// alternative is a row that cannot be shown at all.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            let mut start = 0;
            loop {
                if self.width == 0 || chars.len() - start <= self.width {
                    rows.push(Row {
                        line: index,
                        start,
                        text: chars[start..].iter().collect(),
                    });
                    break;
                }
                let limit = start + self.width;
                // A space exactly where the row runs out breaks there; failing
                // that, the last space before it; failing that, one long word is
                // cut where it runs out.
                let split = if chars[limit] == ' ' {
                    Some(limit)
                } else {
                    (start + 1..limit).rev().find(|&i| chars[i] == ' ')
                };
                let end = split.unwrap_or(limit);
                rows.push(Row {
                    line: index,
                    start,
                    text: chars[start..end].iter().collect(),
                });
                // The space a row broke at belongs to neither row.
                start = if split.is_some() { end + 1 } else { end };
            }
        }
        rows
    }

    /// Where the cursor sits among those rows, as `(row, column)`.
    pub fn cursor_row(&self) -> (usize, usize) {
        let rows = self.rows();
        let index = locate(&rows, self.line, self.col);
        (index, self.col.saturating_sub(rows[index].start))
    }

    fn len(&self, line: usize) -> usize {
        self.lines[line].chars().count()
    }

    fn byte(&self, line: usize, col: usize) -> usize {
        self.lines[line]
            .char_indices()
            .nth(col)
            .map_or(self.lines[line].len(), |(index, _)| index)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte(self.line, self.col);
        self.lines[self.line].insert(at, c);
        self.col += 1;
    }

    /// Type a run of text in at the cursor. A pasted line ending is one new
    /// line however it is written: a terminal hands `\r`, `\n`, or both for the
    /// same key, and two new lines where the paste had one is not what was
    /// copied.
    pub fn insert_str(&mut self, text: &str) {
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    self.newline();
                }
                '\n' => self.newline(),
                _ => self.insert(c),
            }
        }
    }

    pub fn newline(&mut self) {
        let at = self.byte(self.line, self.col);
        let rest = self.lines[self.line].split_off(at);
        self.lines.insert(self.line + 1, rest);
        self.line += 1;
        self.col = 0;
    }

    pub fn backspace(&mut self) {
        if self.col > 0 {
            let at = self.byte(self.line, self.col - 1);
            self.lines[self.line].remove(at);
            self.col -= 1;
        } else if self.line > 0 {
            let tail = self.lines.remove(self.line);
            self.line -= 1;
            self.col = self.len(self.line);
            self.lines[self.line].push_str(&tail);
        }
    }

    pub fn delete(&mut self) {
        if self.col < self.len(self.line) {
            let at = self.byte(self.line, self.col);
            self.lines[self.line].remove(at);
        } else if self.line + 1 < self.lines.len() {
            let tail = self.lines.remove(self.line + 1);
            self.lines[self.line].push_str(&tail);
        }
    }

    pub fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.line > 0 {
            self.line -= 1;
            self.col = self.len(self.line);
        }
    }

    pub fn right(&mut self) {
        if self.col < self.len(self.line) {
            self.col += 1;
        } else if self.line + 1 < self.lines.len() {
            self.line += 1;
            self.col = 0;
        }
    }

    /// Move up one row of the field as drawn, which is not the same as one line
    /// of the text once a line has wrapped. Returns `false` when already on the
    /// top row, which is what lets the caller give the key to something else.
    pub fn up(&mut self) -> bool {
        self.step(-1)
    }

    /// Move down one row as drawn. Returns `false` when already on the last.
    pub fn down(&mut self) -> bool {
        self.step(1)
    }

    fn step(&mut self, delta: isize) -> bool {
        let rows = self.rows();
        let from = locate(&rows, self.line, self.col);
        let to = from as isize + delta;
        if to < 0 || to as usize >= rows.len() {
            return false;
        }
        let column = self.col - rows[from].start;
        let target = &rows[to as usize];
        self.line = target.line;
        self.col = target.start + column.min(target.text.chars().count());
        true
    }

    pub fn home(&mut self) {
        self.col = 0;
    }

    pub fn end(&mut self) {
        self.col = self.len(self.line);
    }

    /// Put the cursor where a click landed, given as a row of the field as
    /// drawn and a column within that row, each clamped to the text that is
    /// actually there.
    pub fn click(&mut self, row: usize, col: usize) {
        let rows = self.rows();
        let target = &rows[row.min(rows.len() - 1)];
        self.line = target.line;
        self.col = target.start + col.min(target.text.chars().count());
    }
}

/// Which row holds a given position in the text: the last one that starts at or
/// before it. Every line has a row starting at zero, so there is always one.
fn locate(rows: &[Row], line: usize, col: usize) -> usize {
    rows.iter()
        .enumerate()
        .filter(|(_, row)| (row.line, row.start) <= (line, col))
        .map(|(index, _)| index)
        .next_back()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(text: &str, width: usize) -> TextField {
        let mut field = TextField::default();
        field.set_text(text);
        field.set_width(width);
        field
    }

    #[test]
    fn setting_text_leaves_the_cursor_at_the_end() {
        let field = field("~/lab/herdr", 40);
        assert!(field.at_end());
    }

    #[test]
    fn a_line_shorter_than_the_field_is_one_row() {
        let rows = field("fix the tests", 40).rows();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].text, "fix the tests");
    }

    #[test]
    fn a_long_line_wraps_at_a_space_and_the_space_belongs_to_neither_row() {
        let rows = field("fix the failing tests", 10).rows();
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, ["fix the", "failing", "tests"]);
        // Every row still reports the one line it came from.
        assert!(rows.iter().all(|row| row.line == 0));
    }

    #[test]
    fn a_word_longer_than_the_field_is_cut_where_it_runs_out() {
        let rows = field("supercalifragilistic", 8).rows();
        let texts: Vec<&str> = rows.iter().map(|row| row.text.as_str()).collect();
        assert_eq!(texts, ["supercal", "ifragili", "stic"]);
    }

    #[test]
    fn a_newline_starts_a_line_rather_than_a_row() {
        let rows = field("first\nsecond", 40).rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].line, 0);
        assert_eq!(rows[1].line, 1);
    }

    #[test]
    fn up_and_down_step_by_rows_on_screen_not_by_lines_in_the_text() {
        let mut field = field("fix the failing tests", 10);
        // The cursor lands at the end, which is the last of three rows.
        assert_eq!(field.cursor_row().0, 2);
        assert!(field.up());
        assert_eq!(field.cursor_row().0, 1);
        assert!(field.up());
        assert_eq!(field.cursor_row().0, 0);
        // The top row has nowhere to go, which is what hands the key on.
        assert!(!field.up());
    }

    #[test]
    fn backspace_at_the_start_of_a_line_joins_it_to_the_one_above() {
        let mut field = field("first\nsecond", 40);
        field.home();
        field.backspace();
        assert_eq!(field.text(), "firstsecond");
    }

    #[test]
    fn a_multi_byte_character_moves_the_cursor_one_step() {
        let mut field = TextField::default();
        field.insert_str("héllo");
        // Four steps back from the end is the far side of the é, not the
        // middle of its two bytes.
        for _ in 0..4 {
            field.left();
        }
        field.backspace();
        assert_eq!(field.text(), "éllo");

        let mut field = TextField::default();
        field.insert_str("héllo");
        field.left();
        field.left();
        field.left();
        field.backspace();
        assert_eq!(field.text(), "hllo", "the é is one backspace, not two");
    }

    #[test]
    fn clearing_the_field_leaves_one_empty_row() {
        let mut field = field("fix the tests", 40);
        field.clear();
        assert!(field.is_empty());
        assert_eq!(field.rows().len(), 1);
    }

    #[test]
    fn a_click_puts_the_cursor_on_the_character_it_landed_on() {
        let mut field = field("fix the failing tests", 10);
        field.click(1, 3);
        field.insert('X');
        assert_eq!(field.text(), "fix the faiXling tests");
    }

    #[test]
    fn a_click_past_the_text_stops_at_what_is_there() {
        let mut field = field("fix the failing tests", 10);
        field.click(0, 99);
        assert_eq!(field.cursor_row(), (0, 7), "the row ends before column 99");
        field.click(9, 99);
        assert_eq!(
            field.cursor_row(),
            (2, 5),
            "and the field ends before row 9"
        );
    }

    #[test]
    fn a_pasted_line_ending_is_one_new_line_however_it_is_written() {
        for pasted in ["first\r\nsecond", "first\nsecond", "first\rsecond"] {
            let mut field = TextField::default();
            field.insert_str(pasted);
            assert_eq!(field.text(), "first\nsecond", "pasting {pasted:?}");
            assert_eq!(field.rows().len(), 2);
        }
    }
}
