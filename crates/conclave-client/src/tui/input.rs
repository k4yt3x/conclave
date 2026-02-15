/// Simple line editor with cursor movement and command history.
pub struct InputLine {
    buffer: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    /// Saved current input when browsing history.
    saved_current: String,
}

impl InputLine {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
            saved_current: String::new(),
        }
    }

    pub fn insert(&mut self, ch: char) {
        self.buffer.insert(self.cursor, ch);
        self.cursor += 1;
    }

    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buffer.remove(self.cursor);
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.buffer.len() {
            self.buffer.remove(self.cursor);
        }
    }

    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor < self.buffer.len() {
            self.cursor += 1;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buffer.len();
    }

    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        match self.history_index {
            None => {
                self.saved_current = self.content();
                self.history_index = Some(self.history.len() - 1);
            }
            Some(0) => return,
            Some(i) => {
                self.history_index = Some(i - 1);
            }
        }
        let entry = &self.history[self.history_index.unwrap()];
        self.buffer = entry.chars().collect();
        self.cursor = self.buffer.len();
    }

    pub fn history_down(&mut self) {
        match self.history_index {
            None => return,
            Some(i) if i + 1 >= self.history.len() => {
                self.history_index = None;
                self.buffer = self.saved_current.chars().collect();
                self.cursor = self.buffer.len();
            }
            Some(i) => {
                self.history_index = Some(i + 1);
                let entry = &self.history[i + 1];
                self.buffer = entry.chars().collect();
                self.cursor = self.buffer.len();
            }
        }
    }

    /// Submit the current input. Returns the content and resets the buffer.
    /// Commands containing credentials (/login, /register) are excluded from history.
    pub fn submit(&mut self) -> String {
        let text = self.content();
        if !text.is_empty() {
            let lower = text.to_lowercase();
            let has_credentials = lower.starts_with("/login ") || lower.starts_with("/register ");
            if !has_credentials {
                self.history.push(text.clone());
            }
        }
        self.buffer.clear();
        self.cursor = 0;
        self.history_index = None;
        self.saved_current.clear();
        text
    }

    pub fn content(&self) -> String {
        self.buffer.iter().collect()
    }

    pub fn cursor_position(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert_and_content() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        assert_eq!(input.content(), "abc");
    }

    #[test]
    fn test_backspace() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.backspace();
        assert_eq!(input.content(), "ab");
    }

    #[test]
    fn test_backspace_empty() {
        let mut input = InputLine::new();
        input.backspace();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn test_delete() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.move_left();
        input.delete();
        assert_eq!(input.content(), "ab");
    }

    #[test]
    fn test_delete_at_end() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.delete();
        assert_eq!(input.content(), "abc");
    }

    #[test]
    fn test_cursor_movement() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        input.insert('d');

        input.home();
        assert_eq!(input.cursor_position(), 0);

        input.end();
        assert_eq!(input.cursor_position(), 4);

        input.move_left();
        assert_eq!(input.cursor_position(), 3);

        input.move_right();
        assert_eq!(input.cursor_position(), 4);
    }

    #[test]
    fn test_cursor_left_at_zero() {
        let mut input = InputLine::new();
        input.insert('a');
        input.home();
        input.move_left();
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_cursor_right_at_end() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.end();
        input.move_right();
        assert_eq!(input.cursor_position(), input.content().len());
    }

    #[test]
    fn test_submit_clears_buffer() {
        let mut input = InputLine::new();
        for ch in "hello".chars() {
            input.insert(ch);
        }
        let result = input.submit();
        assert_eq!(result, "hello");
        assert!(input.is_empty());
    }

    #[test]
    fn test_submit_adds_to_history() {
        let mut input = InputLine::new();
        for ch in "hello".chars() {
            input.insert(ch);
        }
        input.submit();
        input.history_up();
        assert_eq!(input.content(), "hello");
    }

    #[test]
    fn test_submit_excludes_login_from_history() {
        let mut input = InputLine::new();
        for ch in "/login user pass".chars() {
            input.insert(ch);
        }
        input.submit();
        input.history_up();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn test_submit_excludes_register_from_history() {
        let mut input = InputLine::new();
        for ch in "/register user pass".chars() {
            input.insert(ch);
        }
        input.submit();
        input.history_up();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn test_history_navigation() {
        let mut input = InputLine::new();
        for ch in "first".chars() {
            input.insert(ch);
        }
        input.submit();
        for ch in "second".chars() {
            input.insert(ch);
        }
        input.submit();

        input.history_up();
        assert_eq!(input.content(), "second");

        input.history_up();
        assert_eq!(input.content(), "first");

        input.history_down();
        assert_eq!(input.content(), "second");

        input.history_down();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn test_history_up_empty() {
        let mut input = InputLine::new();
        input.history_up();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn test_history_down_no_navigation() {
        let mut input = InputLine::new();
        input.history_down();
        assert_eq!(input.content(), "");
    }

    #[test]
    fn test_is_empty() {
        let mut input = InputLine::new();
        assert!(input.is_empty());
        input.insert('a');
        assert!(!input.is_empty());
    }

    #[test]
    fn test_insert_at_cursor_position() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('c');
        input.move_left();
        input.insert('b');
        assert_eq!(input.content(), "abc");
    }

    #[test]
    fn test_cursor_position_tracking() {
        let mut input = InputLine::new();
        input.insert('a');
        input.insert('b');
        input.insert('c');
        assert_eq!(input.cursor_position(), 3);
        input.move_left();
        assert_eq!(input.cursor_position(), 2);
    }

    #[test]
    fn test_submit_empty_not_added_to_history() {
        let mut input = InputLine::new();
        input.submit();
        input.history_up();
        assert_eq!(input.content(), "");
    }
}
