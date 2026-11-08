//! An editable line of text: caret, selection, clipboard and password masking.

/// One hidden character. Only the count of them shows, so a key's length is all that's visible.
pub const MASK: char = '•';

/// A field is a host, a port, or an API key. Anything longer is either a mistake or a paste
/// of something that was never meant for this box, and a very long string makes every field
/// interaction slow.
const MAX_FIELD_CHARS: usize = 512;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Field {
    text: String,
    caret: usize,
    anchor: usize,

    /// How far the text is scrolled to keep the caret visible, in logical units.
    pub view: f32,
    pub password: bool,
}

impl Field {
    pub fn new(text: &str, password: bool) -> Self {
        let length = text.chars().count();
        Self { text: text.to_string(), caret: length, anchor: length, view: 0.0, password }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    /// What to draw, which hides the real text for a password.
    pub fn display(&self) -> String {
        if self.password { MASK.to_string().repeat(self.len()) } else { self.text.clone() }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    pub fn selection(&self) -> Option<(usize, usize)> {
        (self.caret != self.anchor).then(|| (self.caret.min(self.anchor), self.caret.max(self.anchor)))
    }

    fn byte(&self, index: usize) -> usize {
        self.text.char_indices().nth(index).map_or(self.text.len(), |(byte, _)| byte)
    }

    pub fn move_to(&mut self, index: usize, extend: bool) {
        self.caret = index.min(self.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    pub fn left(&mut self, extend: bool) {
        match self.selection() {
            Some((start, _)) if !extend => self.move_to(start, false),
            _ => self.move_to(self.caret.saturating_sub(1), extend),
        }
    }

    pub fn right(&mut self, extend: bool) {
        match self.selection() {
            Some((_, end)) if !extend => self.move_to(end, false),
            _ => self.move_to(self.caret + 1, extend),
        }
    }

    pub fn home(&mut self, extend: bool) {
        self.move_to(0, extend);
    }

    pub fn end(&mut self, extend: bool) {
        self.move_to(self.len(), extend);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.caret = self.len();
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };

        let (from, to) = (self.byte(start), self.byte(end));
        self.text.replace_range(from..to, "");
        self.move_to(start, false);
        true
    }

    pub fn insert(&mut self, typed: &str) {
        self.delete_selection();

        // One line only, so a pasted key with a trailing newline still works.
        let room = MAX_FIELD_CHARS.saturating_sub(self.text.chars().count());
        let clean: String = typed.lines().next().unwrap_or("").chars().filter(|c| !c.is_control()).take(room).collect();
        let at = self.byte(self.caret);
        self.text.insert_str(at, &clean);
        self.move_to(self.caret + clean.chars().count(), false);
    }

    pub fn backspace(&mut self) {
        if !self.delete_selection() && self.caret > 0 {
            self.move_to(self.caret - 1, false);
            let at = self.byte(self.caret);
            self.text.remove(at);
        }
    }

    pub fn delete(&mut self) {
        if !self.delete_selection() && self.caret < self.len() {
            let at = self.byte(self.caret);
            self.text.remove(at);
        }
    }

    /// The selected text for the clipboard. A password can be pasted in but never copied out.
    pub fn copy(&self) -> Option<String> {
        let (start, end) = self.selection()?;
        (!self.password).then(|| self.text.chars().skip(start).take(end - start).collect())
    }

    pub fn cut(&mut self) -> Option<String> {
        let copied = self.copy()?;
        self.delete_selection();
        Some(copied)
    }
}
