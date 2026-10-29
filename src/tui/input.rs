use unicode_width::UnicodeWidthChar;

#[derive(Debug, Clone, Default)]
pub struct InputBuffer {
    text: String,
    cursor: usize,
}

impl InputBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    pub fn insert_char(&mut self, ch: char) {
        let ch = if ch == '\t' { ' ' } else { ch };
        self.text.insert(self.cursor, ch);
        self.cursor += ch.len_utf8();
    }

    pub fn insert_str(&mut self, value: &str) {
        let value = value.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &value);
        self.cursor += value.len();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.prev_boundary(self.cursor);
        self.text.drain(prev..self.cursor);
        self.cursor = prev;
    }

    pub fn delete(&mut self) {
        if self.cursor >= self.text.len() {
            return;
        }
        let next = self.next_boundary(self.cursor);
        self.text.drain(self.cursor..next);
    }

    pub fn delete_word(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut start = self.cursor;
        while start > 0 {
            let ch = self.text[..start].chars().next_back().unwrap_or(' ');
            if !ch.is_whitespace() {
                break;
            }
            start -= ch.len_utf8();
        }
        while start > 0 {
            let ch = self.text[..start].chars().next_back().unwrap_or(' ');
            if ch.is_whitespace() {
                break;
            }
            start -= ch.len_utf8();
        }
        self.text.drain(start..self.cursor);
        self.cursor = start;
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary(self.cursor);
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary(self.cursor);
    }

    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
    }

    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.text.len());
    }

    pub fn up(&mut self) -> bool {
        let start = self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        if start == 0 {
            return false;
        }
        let column = self.text[start..self.cursor].chars().count();
        let prev_end = start - 1;
        let prev_start = self.text[..prev_end]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let mut position = prev_start;
        for (count, (offset, ch)) in self.text[prev_start..prev_end].char_indices().enumerate() {
            if count >= column {
                break;
            }
            position = prev_start + offset + ch.len_utf8();
        }
        self.cursor = position.min(prev_end);
        true
    }

    pub fn down(&mut self) -> bool {
        let start = self.text[..self.cursor]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let line_end = self.text[self.cursor..]
            .find('\n')
            .map(|index| self.cursor + index)
            .unwrap_or(self.text.len());
        if line_end >= self.text.len() {
            return false;
        }
        let column = self.text[start..self.cursor].chars().count();
        let next_start = line_end + 1;
        let next_end = self.text[next_start..]
            .find('\n')
            .map(|index| next_start + index)
            .unwrap_or(self.text.len());
        let mut position = next_start;
        for (count, (offset, ch)) in self.text[next_start..next_end].char_indices().enumerate() {
            if count >= column {
                break;
            }
            position = next_start + offset + ch.len_utf8();
        }
        self.cursor = position;
        true
    }

    pub fn line_col(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        let row = before.matches('\n').count();
        let line_start = before.rfind('\n').map(|index| index + 1).unwrap_or(0);
        let col = self.text[line_start..self.cursor]
            .chars()
            .map(|ch| ch.width().unwrap_or(0))
            .sum();
        (row, col)
    }

    pub fn line_count(&self) -> usize {
        self.text.split('\n').count().max(1)
    }

    pub fn lines(&self) -> Vec<String> {
        self.text.split('\n').map(str::to_string).collect()
    }

    fn prev_boundary(&self, index: usize) -> usize {
        self.text[..index]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0)
    }

    fn next_boundary(&self, index: usize) -> usize {
        self.text[index..]
            .char_indices()
            .nth(1)
            .map(|(offset, _)| index + offset)
            .unwrap_or(self.text.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_at_char_boundaries() {
        let mut input = InputBuffer::new();
        input.insert_str("héllo\nworld");
        input.home();
        input.insert_char('>');
        assert_eq!(input.text(), "héllo\n>world");
        input.end();
        input.backspace();
        assert_eq!(input.text(), "héllo\n>worl");
        input.home();
        input.up();
        assert_eq!(input.line_col().0, 0);
    }

    #[test]
    fn deletes_words() {
        let mut input = InputBuffer::new();
        input.insert_str("one two three");
        input.delete_word();
        assert_eq!(input.text(), "one two ");
    }
}
