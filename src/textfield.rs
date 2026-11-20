//! A line of text being typed in an editor: where the caret is, what is selected, and the keys
//! that move them and change the text. No mod API types here.
//!
//! Positions count characters, not bytes, so any letters may be typed. A selection is the text
//! between the anchor and the caret (Shift with a move, Ctrl+A, a double click).

/// Where the caret is in a text, and where a selection began (none: nothing selected).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Caret {
    pub at: usize,
    pub anchor: Option<usize>,
}

/// The keys that move the caret or change the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Left,
    Right,
    Home,
    End,
    Back,
    Delete,
    All,
}

pub fn len(text: &str) -> usize {
    text.chars().count()
}

/// Where character `i` begins, in bytes (the end past the last).
fn byte(text: &str, i: usize) -> usize {
    text.char_indices().nth(i).map_or(text.len(), |(b, _)| b)
}

/// Characters `from..to` of `text`.
pub fn slice(text: &str, from: usize, to: usize) -> &str {
    let (a, b) = (byte(text, from.min(to)), byte(text, to.max(from)));
    &text[a..b]
}

impl Caret {
    /// At the end of `text`, nothing selected.
    pub fn end(text: &str) -> Caret {
        Caret { at: len(text), anchor: None }
    }

    /// Kept within `text` (it may have changed under it).
    pub fn clamp(&mut self, text: &str) {
        let n = len(text);
        self.at = self.at.min(n);
        self.anchor = self.anchor.map(|a| a.min(n)).filter(|a| *a != self.at);
    }

    /// The selection, from its first character to past its last (none: nothing selected).
    pub fn range(&self) -> Option<(usize, usize)> {
        let a = self.anchor?;
        (a != self.at).then(|| (a.min(self.at), a.max(self.at)))
    }

    /// Moves to `to`; with `select`, the selection grows from where it was.
    fn go(&mut self, to: usize, select: bool) {
        if select {
            self.anchor.get_or_insert(self.at);
        } else {
            self.anchor = None;
        }
        self.at = to;
        if self.anchor == Some(self.at) {
            self.anchor = None;
        }
    }
}

/// The selected text, if any.
pub fn selected(text: &str, c: &Caret) -> Option<String> {
    c.range().map(|(a, b)| slice(text, a, b).to_string())
}

/// Takes out characters `from..to`; the caret goes where they were.
fn remove(text: &mut String, c: &mut Caret, from: usize, to: usize) {
    let (a, b) = (byte(text, from), byte(text, to));
    text.replace_range(a..b, "");
    c.at = from;
    c.anchor = None;
}

/// Takes out the selection, if any: whether there was one.
fn remove_selected(text: &mut String, c: &mut Caret) -> bool {
    match c.range() {
        Some((a, b)) => {
            remove(text, c, a, b);
            true
        }
        None => false,
    }
}

/// Types `s` at the caret (over the selection), as much as fits in `max` characters; control
/// characters are left out (a pasted line break too).
pub fn insert(text: &mut String, c: &mut Caret, s: &str, max: usize) {
    c.clamp(text);
    remove_selected(text, c);
    let room = max.saturating_sub(len(text));
    let add: String = s.chars().filter(|ch| !ch.is_control()).take(room).collect();
    if add.is_empty() {
        return;
    }
    let at = byte(text, c.at);
    text.insert_str(at, &add);
    c.at += len(&add);
}

/// The selection taken out (for Ctrl+X).
pub fn cut(text: &mut String, c: &mut Caret) -> Option<String> {
    c.clamp(text);
    let s = selected(text, c)?;
    remove_selected(text, c);
    Some(s)
}

/// Where the word before `at` begins (spaces before it skipped).
fn word_back(chars: &[char], at: usize) -> usize {
    let mut i = at;
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

/// Where the word after `at` ends (spaces after it skipped).
fn word_on(chars: &[char], at: usize) -> usize {
    let mut i = at;
    while i < chars.len() && chars[i].is_whitespace() {
        i += 1;
    }
    while i < chars.len() && !chars[i].is_whitespace() {
        i += 1;
    }
    i
}

/// Key `k` in `text`: `select` (Shift) grows the selection, `word` (Ctrl) goes a word at a time.
/// Whether the text changed.
pub fn key(text: &mut String, c: &mut Caret, k: Key, select: bool, word: bool) -> bool {
    c.clamp(text);
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    match k {
        Key::Left => {
            let to = match c.range() {
                Some((a, _)) if !select => a,
                _ if word => word_back(&chars, c.at),
                _ => c.at.saturating_sub(1),
            };
            c.go(to, select);
            false
        }
        Key::Right => {
            let to = match c.range() {
                Some((_, b)) if !select => b,
                _ if word => word_on(&chars, c.at),
                _ => (c.at + 1).min(n),
            };
            c.go(to, select);
            false
        }
        Key::Home => {
            c.go(0, select);
            false
        }
        Key::End => {
            c.go(n, select);
            false
        }
        Key::All => {
            c.anchor = (n > 0).then_some(0);
            c.at = n;
            false
        }
        Key::Back => {
            if remove_selected(text, c) {
                return true;
            }
            let from = if word { word_back(&chars, c.at) } else { c.at.saturating_sub(1) };
            if from == c.at {
                return false;
            }
            remove(text, c, from, c.at);
            true
        }
        Key::Delete => {
            if remove_selected(text, c) {
                return true;
            }
            let to = if word { word_on(&chars, c.at) } else { (c.at + 1).min(n) };
            if to == c.at {
                return false;
            }
            let at = c.at;
            remove(text, c, at, to);
            true
        }
    }
}

/// The first character shown of a field `fits` wide, so that the caret is in sight: from
/// `start` as it was, moved as little as it takes, and back as far as the room allows (so a
/// field never shows less than it could).
pub fn window(text: &str, caret: usize, start: usize, fits: &dyn Fn(&str) -> bool) -> usize {
    let n = len(text);
    let caret = caret.min(n);
    let mut s = start.min(caret);
    // The caret past the right edge: the start comes after it.
    while s < caret && !fits(slice(text, s, caret)) {
        s += 1;
    }
    // Room at the right: as much of the start as fits.
    while s > 0 && fits(slice(text, s - 1, n)) {
        s -= 1;
    }
    s
}

/// The character boundary nearest `x` in a field showing `text` from `start`, where `width` is how
/// wide a piece of text is drawn.
pub fn at_x(text: &str, start: usize, x: f32, width: &dyn Fn(&str) -> f32) -> usize {
    let n = len(text);
    let mut best = (start.min(n), f32::MAX);
    for i in start.min(n)..=n {
        let d = (width(slice(text, start, i)) - x).abs();
        if d < best.1 {
            best = (i, d);
        }
        if width(slice(text, start, i)) > x + 8.0 {
            break;
        }
    }
    best.0
}

/// The word about character `i` (for a double click): its start and end.
pub fn word_at(text: &str, i: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    let i = i.min(chars.len());
    let mut a = i;
    while a > 0 && !chars[a - 1].is_whitespace() {
        a -= 1;
    }
    let mut b = i;
    while b < chars.len() && !chars[b].is_whitespace() {
        b += 1;
    }
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_in_the_middle() {
        let mut t = String::from("Hello world");
        let mut c = Caret::end(&t);
        key(&mut t, &mut c, Key::Home, false, false);
        key(&mut t, &mut c, Key::Right, false, true);
        insert(&mut t, &mut c, ",", 100);
        assert_eq!(t, "Hello, world");
        // Shift+End selects the rest; typing replaces it.
        key(&mut t, &mut c, Key::End, true, false);
        assert_eq!(selected(&t, &c).as_deref(), Some(" world"));
        insert(&mut t, &mut c, " there", 100);
        assert_eq!(t, "Hello, there");
        // Ctrl+Backspace takes the word before; Delete the letter after.
        key(&mut t, &mut c, Key::Back, false, true);
        assert_eq!(t, "Hello, ");
        key(&mut t, &mut c, Key::Home, false, false);
        key(&mut t, &mut c, Key::Delete, false, false);
        assert_eq!(t, "ello, ");
        // Not over the most.
        let mut c = Caret::end(&t);
        insert(&mut t, &mut c, "abcdef", 8);
        assert_eq!(t, "ello, ab");
        // Ctrl+A and cut.
        key(&mut t, &mut c, Key::All, false, false);
        assert_eq!(cut(&mut t, &mut c).as_deref(), Some("ello, ab"));
        assert!(t.is_empty());
    }

    #[test]
    fn any_letters() {
        let mut t = String::from("Привет мир");
        let mut c = Caret::end(&t);
        key(&mut t, &mut c, Key::Left, false, true);
        assert_eq!(c.at, 7);
        insert(&mut t, &mut c, "ё ", 100);
        assert_eq!(t, "Привет ё мир");
        key(&mut t, &mut c, Key::Back, false, false);
        key(&mut t, &mut c, Key::Back, false, false);
        assert_eq!(t, "Привет мир");
        // A caret left past a text that got shorter is put back in it.
        let mut c = Caret { at: 99, anchor: Some(50) };
        c.clamp(&t);
        assert_eq!(c, Caret { at: 10, anchor: None });
        assert_eq!(word_at(&t, 2), (0, 6));
    }

    #[test]
    fn the_caret_stays_in_sight() {
        // Each letter is 1 wide, the field 5.
        let fits = |s: &str| len(s) <= 5;
        let text = "abcdefghij";
        assert_eq!(window(text, 10, 0, &fits), 5);
        assert_eq!(window(text, 2, 5, &fits), 2);
        assert_eq!(window("abc", 3, 2, &fits), 0, "short: all of it");
        let width = |s: &str| len(s) as f32;
        assert_eq!(at_x(text, 5, 2.4, &width), 7);
    }
}
