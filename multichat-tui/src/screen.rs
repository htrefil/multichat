use crossterm::cursor::MoveTo;
use crossterm::event::{self, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Color, Print, PrintStyledContent, ResetColor, SetForegroundColor, Stylize};
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};
use log::Level;
use std::collections::VecDeque;
use std::io::{self, Error, Stdout, Write};
use std::mem;

pub struct Screen {
    stdout: Stdout,
    size: (u16, u16),
    rows: VecDeque<Row>,
    input: String,
}

impl Screen {
    pub fn new() -> Result<Self, Error> {
        // Enter alternate screen so that whatever state the users shell was in
        // will not be trashed. This is what vim does, for example.
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnterAlternateScreen)?;

        let size = terminal::size()?;
        terminal::enable_raw_mode()?;

        Ok(Self {
            stdout,
            size,
            rows: VecDeque::new(),
            input: String::new(),
        })
    }

    /// Write an entry into the "log". It's used for both chat messages and other logs.
    pub fn write(&mut self, level: Level, source: Option<String>, message: String) {
        const MAX_ROWS: usize = 256;

        if self.rows.len() == MAX_ROWS {
            self.rows.pop_front();
        }

        self.rows.push_back(Row {
            level,
            source,
            message,
        });
    }

    pub fn event(&mut self, event: event::Event) -> Option<Event> {
        match event {
            event::Event::Key(event) => {
                match event.code {
                    KeyCode::Char('c' | 'C') if event.modifiers == KeyModifiers::CONTROL => {
                        return Some(Event::Exit);
                    }
                    KeyCode::Char(c) => {
                        self.input.push(c);
                    }
                    KeyCode::Backspace => {
                        self.input.pop();
                    }
                    KeyCode::Enter => {
                        return Some(Event::Input(mem::take(&mut self.input)));
                    }
                    _ => {}
                }

                None
            }
            event::Event::Mouse(_) => None,
            event::Event::Resize(n, _) | event::Event::Resize(_, n) if n <= 1 => Some(Event::Exit),
            event::Event::Resize(width, height) => {
                self.size = (width, height);
                None
            }
        }
    }

    pub fn render(&mut self) -> Result<(), Error> {
        // TODO: Render only what's necessary to re-render from the last frame.
        //       Screen::{input,write} should note what, if anything, changed, so that we
        //       don't do it all over again for no good reason.

        // TODO: Doesn't compile, multiple mutable borrows.
        todo!()

        // let width = self.size.0 as usize;
        // let height = self.size.1 as usize;

        // crossterm::queue!(self.stdout, MoveTo(0, 0))?;

        // for row in self.rows.iter().rev().take(width - 1) {
        //     let (c, color) = match row.level {
        //         Level::Error => ('-', Color::Red),
        //         Level::Warn => ('!', Color::Yellow),
        //         Level::Info => ('+', Color::Green),
        //         Level::Debug => ('?', Color::Blue),
        //         Level::Trace => ('?', Color::DarkBlue),
        //     };

        //     crossterm::queue!(self.stdout, PrintStyledContent(c.with(color)), Print(" "))?;

        //     if let Some(source) = &row.source {
        //         crossterm::queue!(
        //             self.stdout,
        //             PrintStyledContent(source.as_str().with(Color::White)),
        //             Print(" ")
        //         )?;
        //     }
        // }

        // let stdout = &mut self.stdout;
        // crossterm::queue!(stdout, MoveTo(0, self.size.1 - 1))?;
        // crossterm::queue!(stdout, Print(&self.input))?;

        // crossterm::execute!(stdout)?;

        // Ok(())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        // Doesn't seem like we can do much more than silently ignore the errors.
        // If we panic the error message won't most likely be correctly printed anyway.
        let _ = terminal::disable_raw_mode();
        let _ = crossterm::execute!(std::io::stdout(), LeaveAlternateScreen);
    }
}

pub enum Event {
    /// Ctrl+C pressed.
    Exit,
    /// Enter was pressed.
    Input(String),
}

struct Row {
    level: Level,
    source: Option<String>,
    message: String,
}
