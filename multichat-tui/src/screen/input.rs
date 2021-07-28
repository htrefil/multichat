use crossterm::cursor::MoveTo;
use crossterm::style::Print;
use crossterm::terminal::{Clear, ClearType};
use std::collections::VecDeque;
use std::io::{Error, Write};

const MAX_HISTORY: usize = 256;

pub struct Input {
    history: VecDeque<Vec<char>>,
    cursor: usize,
    kind: InputKind,
    changed: bool,
    height: u16,
}

impl Input {
    pub fn new() -> Self {
        Self {
            history: VecDeque::new(),
            cursor: 0,
            kind: InputKind::Owned(Vec::new()),
            changed: true,
            height: 0,
        }
    }

    pub fn prev_history(&mut self) {
        if self.history.len() == 0 {
            return;
        }

        self.kind = match self.kind {
            InputKind::History(idx) => InputKind::History(idx.wrapping_sub(1) % self.history.len()),
            InputKind::Owned(_) => InputKind::History(0),
        };

        self.cursor = self.as_ref().len();
        self.changed = true;
    }

    pub fn next_history(&mut self) {
        if self.history.len() == 0 {
            return;
        }

        self.kind = match self.kind {
            InputKind::History(idx) => InputKind::History((idx + 1) % self.history.len()),
            InputKind::Owned(_) => InputKind::History(0),
        };

        self.cursor = self.as_ref().len();
        self.changed = true;
    }

    pub fn prev_char(&mut self) {
        let cursor = self.cursor.saturating_sub(1);
        self.changed = self.cursor != cursor;
        self.cursor = cursor;
    }

    pub fn next_char(&mut self) {
        let cursor = (self.cursor + 1).min(self.as_ref().len());
        self.changed = self.cursor != cursor;
        self.cursor = cursor;
    }

    pub fn first_char(&mut self) {
        self.changed = self.cursor != 0;
        self.cursor = 0;
    }

    pub fn last_char(&mut self) {
        let cursor = self.as_ref().len();
        self.changed = self.cursor != cursor;
        self.cursor = cursor;
    }

    pub fn input(&mut self, c: char) {
        let cursor = self.cursor;

        self.as_mut().insert(cursor, c);
        self.cursor += 1;
        self.changed = true;
    }

    pub fn enter(&mut self) -> String {
        let data: Vec<_> = self.as_ref().iter().copied().collect();

        if self.history.len() == MAX_HISTORY {
            self.history.pop_front();
        }

        self.history.push_back(data.clone());
        self.kind = InputKind::Owned(Vec::new());
        self.cursor = 0;
        self.changed = true;

        data.into_iter().collect()
    }

    pub fn erase(&mut self) {
        if self.as_ref().is_empty() || self.cursor == 0 {
            return;
        }

        let cursor = self.cursor;
        let input = self.as_mut();

        input.remove(cursor - 1);
        self.cursor -= 1;
        self.changed = true;
    }

    pub fn as_ref(&self) -> &[char] {
        match &self.kind {
            InputKind::History(idx) => &self.history[*idx],
            InputKind::Owned(data) => data,
        }
    }

    pub fn render(&mut self, mut writer: impl Write, height: u16) -> Result<(), Error> {
        if !self.changed && self.height == height {
            return Ok(());
        }

        self.changed = false;
        self.height = height;

        crossterm::queue!(writer, MoveTo(0, height))?;
        crossterm::queue!(writer, Clear(ClearType::CurrentLine))?;

        for c in self.as_ref() {
            crossterm::queue!(writer, Print(c))?;
        }

        crossterm::queue!(writer, MoveTo(self.cursor as u16, height - 1))?;

        Ok(())
    }

    fn as_mut(&mut self) -> &mut Vec<char> {
        self.kind = match std::mem::replace(&mut self.kind, InputKind::History(0)) {
            InputKind::History(idx) => InputKind::Owned(self.history[idx].clone()),
            InputKind::Owned(data) => InputKind::Owned(data),
        };

        match &mut self.kind {
            InputKind::Owned(data) => data,
            _ => unreachable!(),
        }
    }
}

enum InputKind {
    History(usize),
    Owned(Vec<char>),
}
