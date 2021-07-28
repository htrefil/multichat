use crossterm::cursor::MoveTo;
use crossterm::style::{Color, Print, PrintStyledContent, Stylize};
use crossterm::terminal::{Clear, ClearType};
use std::collections::VecDeque;
use std::io::{Error, Write};

const MAX_ROWS: usize = 256;

pub struct Log {
    rows: VecDeque<(Level, String)>,
    changed: bool,
    height: u16,
}

impl Log {
    pub fn new() -> Self {
        Self {
            rows: VecDeque::new(),
            changed: true,
            height: 0,
        }
    }

    pub fn log(&mut self, level: Level, contents: String) {
        if self.rows.len() == MAX_ROWS {
            self.rows.pop_front();
        }

        self.rows.push_back((level, contents));
        self.changed = true;
    }

    pub fn render(&mut self, mut writer: impl Write, height: u16) -> Result<(), Error> {
        if !self.changed && self.height == height {
            return Ok(());
        }

        self.changed = false;
        self.height = height;

        for (i, (level, contents)) in self.last((height - 1) as usize).enumerate() {
            crossterm::queue!(&mut writer, MoveTo(0, i as u16))?;
            crossterm::queue!(&mut writer, Clear(ClearType::CurrentLine))?;

            let (prefix, color) = match level {
                Level::Error => ("[-]", Color::Red),
                Level::Warn => ("[!]", Color::Yellow),
                Level::Info => ("[+]", Color::Green),
            };

            crossterm::queue!(
                &mut writer,
                PrintStyledContent(prefix.with(color)),
                Print(" "),
                Print(contents),
            )?;
        }

        Ok(())
    }

    fn last(&self, num: usize) -> impl Iterator<Item = (Level, &str)> {
        let offset = if self.rows.len() >= num {
            self.rows.len() - num
        } else {
            0
        };

        self.rows
            .range(offset..)
            .map(|(level, contents)| (*level, contents.as_str()))
    }
}

#[derive(Clone, Copy)]
pub enum Level {
    Info,
    Warn,
    Error,
}
