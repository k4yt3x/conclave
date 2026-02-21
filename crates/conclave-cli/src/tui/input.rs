/// Simple line editor with cursor movement and command history.
pub struct InputLine {
    buffer: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    /// Saved current input when browsing history.
    saved_current: String,
}

fn is_word_char(character: char) -> bool {
    character.is_alphanumeric() || character == '_'
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

    pub fn move_word_left(&mut self) {
        while self.cursor > 0 && !is_word_char(self.buffer[self.cursor - 1]) {
            self.cursor -= 1;
        }
        while self.cursor > 0 && is_word_char(self.buffer[self.cursor - 1]) {
            self.cursor -= 1;
        }
    }

    pub fn move_word_right(&mut self) {
        let length = self.buffer.len();
        while self.cursor < length && is_word_char(self.buffer[self.cursor]) {
            self.cursor += 1;
        }
        while self.cursor < length && !is_word_char(self.buffer[self.cursor]) {
            self.cursor += 1;
        }
    }

    pub fn kill_to_end(&mut self) {
        self.buffer.truncate(self.cursor);
    }

    pub fn kill_to_start(&mut self) {
        self.buffer.drain(..self.cursor);
        self.cursor = 0;
    }

    pub fn kill_word_backward(&mut self) {
        let original = self.cursor;
        while self.cursor > 0 && !is_word_char(self.buffer[self.cursor - 1]) {
            self.cursor -= 1;
        }
        while self.cursor > 0 && is_word_char(self.buffer[self.cursor - 1]) {
            self.cursor -= 1;
        }
        self.buffer.drain(self.cursor..original);
    }

    pub fn delete_word_forward(&mut self) {
        let start = self.cursor;
        let length = self.buffer.len();
        let mut end = self.cursor;
        while end < length && is_word_char(self.buffer[end]) {
            end += 1;
        }
        while end < length && !is_word_char(self.buffer[end]) {
            end += 1;
        }
        self.buffer.drain(start..end);
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

    fn input_from(text: &str) -> InputLine {
        let mut input = InputLine::new();
        for ch in text.chars() {
            input.insert(ch);
        }
        input
    }

    #[test]
    fn test_move_word_left() {
        let mut input = input_from("hello world");
        input.move_word_left();
        assert_eq!(input.cursor_position(), 6);
        input.move_word_left();
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_move_word_left_at_start() {
        let mut input = input_from("hello");
        input.home();
        input.move_word_left();
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_move_word_left_multiple_spaces() {
        let mut input = input_from("hello   world");
        input.move_word_left();
        assert_eq!(input.cursor_position(), 8);
    }

    #[test]
    fn test_move_word_right() {
        let mut input = input_from("hello world");
        input.home();
        input.move_word_right();
        assert_eq!(input.cursor_position(), 6);
        input.move_word_right();
        assert_eq!(input.cursor_position(), 11);
    }

    #[test]
    fn test_move_word_right_at_end() {
        let mut input = input_from("hello");
        input.move_word_right();
        assert_eq!(input.cursor_position(), 5);
    }

    #[test]
    fn test_move_word_right_multiple_spaces() {
        let mut input = input_from("hello   world");
        input.home();
        input.move_word_right();
        assert_eq!(input.cursor_position(), 8);
    }

    #[test]
    fn test_kill_to_end() {
        let mut input = input_from("hello world");
        input.home();
        for _ in 0..5 {
            input.move_right();
        }
        input.kill_to_end();
        assert_eq!(input.content(), "hello");
        assert_eq!(input.cursor_position(), 5);
    }

    #[test]
    fn test_kill_to_end_at_end() {
        let mut input = input_from("hello");
        input.kill_to_end();
        assert_eq!(input.content(), "hello");
    }

    #[test]
    fn test_kill_to_start() {
        let mut input = input_from("hello world");
        input.home();
        for _ in 0..5 {
            input.move_right();
        }
        input.kill_to_start();
        assert_eq!(input.content(), " world");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_kill_to_start_at_start() {
        let mut input = input_from("hello");
        input.home();
        input.kill_to_start();
        assert_eq!(input.content(), "hello");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_kill_word_backward() {
        let mut input = input_from("hello world");
        input.kill_word_backward();
        assert_eq!(input.content(), "hello ");
        assert_eq!(input.cursor_position(), 6);
    }

    #[test]
    fn test_kill_word_backward_multiple_spaces() {
        let mut input = input_from("hello   world");
        input.kill_word_backward();
        assert_eq!(input.content(), "hello   ");
        assert_eq!(input.cursor_position(), 8);
    }

    #[test]
    fn test_kill_word_backward_at_start() {
        let mut input = input_from("hello");
        input.home();
        input.kill_word_backward();
        assert_eq!(input.content(), "hello");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_delete_word_forward() {
        let mut input = input_from("hello world");
        input.home();
        input.delete_word_forward();
        assert_eq!(input.content(), "world");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_delete_word_forward_at_end() {
        let mut input = input_from("hello");
        input.delete_word_forward();
        assert_eq!(input.content(), "hello");
    }

    #[test]
    fn test_delete_word_forward_with_punctuation() {
        let mut input = input_from("hello, world");
        input.home();
        input.delete_word_forward();
        assert_eq!(input.content(), "world");
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_word_operations_with_underscores() {
        let mut input = input_from("hello_world foo");
        input.move_word_left();
        assert_eq!(input.cursor_position(), 12);
        input.move_word_left();
        assert_eq!(input.cursor_position(), 0);
    }

    #[test]
    fn test_word_operations_empty_buffer() {
        let mut input = InputLine::new();
        input.move_word_left();
        assert_eq!(input.cursor_position(), 0);
        input.move_word_right();
        assert_eq!(input.cursor_position(), 0);
        input.kill_word_backward();
        assert_eq!(input.content(), "");
        input.delete_word_forward();
        assert_eq!(input.content(), "");
        input.kill_to_end();
        assert_eq!(input.content(), "");
        input.kill_to_start();
        assert_eq!(input.content(), "");
    }
}
