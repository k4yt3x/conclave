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
