mod input;
mod log;

pub use log::Level;

use crossterm::cursor::MoveTo;
use crossterm::event::{self, Event as TermEvent, EventStream, KeyCode, KeyEvent, KeyModifiers};
use crossterm::style::{Color, Print, PrintStyledContent, ResetColor, SetForegroundColor, Stylize};
use crossterm::terminal::{
    self, Clear, ClearType, DisableLineWrap, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures::stream::StreamExt;
use input::Input;
use log::Log;
use std::collections::VecDeque;
use std::io::{self, Error, Stdout};

pub struct Screen {
    stdout: Stdout,
    stream: EventStream,
    height: u16,
    event: Option<TermEvent>,
    log: Log,
    input: Input,
}

impl Screen {
    pub fn new() -> Result<Self, Error> {
        // Enter alternate screen so that whatever state the users shell was in
        // will not be trashed. This is what vim does, for example.
        let mut stdout = io::stdout();
        crossterm::execute!(stdout, EnterAlternateScreen)?;
        crossterm::execute!(stdout, DisableLineWrap)?;

        let (width, height) = terminal::size()?;
        terminal::enable_raw_mode()?;

        Ok(Self {
            stdout,
            stream: EventStream::new(),
            height,
            event: Some(TermEvent::Resize(width, height)),
            log: Log::new(),
            input: Input::new(),
        })
    }

    pub fn log(&mut self, level: Level, contents: String) {
        self.log.log(level, contents);
    }

    pub async fn process(&mut self) -> Result<Option<Event>, Error> {
        let event = match self.event.take() {
            Some(event) => event,
            None => self.stream.next().await.unwrap()?,
        };

        let event = match event {
            TermEvent::Key(key) => match key.code {
                KeyCode::Char('c' | 'C') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    Some(Event::Quit)
                }
                KeyCode::Char(c) => {
                    self.input.input(c);
                    None
                }
                KeyCode::Backspace => {
                    self.input.erase();
                    None
                }
                KeyCode::End => {
                    self.input.last_char();
                    None
                }
                KeyCode::Home => {
                    self.input.first_char();
                    None
                }
                KeyCode::Enter => Some(Event::Input(self.input.enter())),
                KeyCode::Left => {
                    self.input.prev_char();
                    None
                }
                KeyCode::Right => {
                    self.input.next_char();
                    None
                }
                KeyCode::Up => {
                    self.input.prev_history();
                    None
                }
                KeyCode::Down => {
                    self.input.next_history();
                    None
                }
                _ => None,
            },
            TermEvent::Mouse(_) => None,
            TermEvent::Resize(0..=1, _) | TermEvent::Resize(_, 0..=1) => Some(Event::Quit),
            TermEvent::Resize(_, height) => {
                self.height = height;
                None
            }
        };

        Ok(event)
    }

    pub fn render(&mut self) -> Result<(), Error> {
        self.log.render(&mut self.stdout, self.height)?;
        self.input.render(&mut self.stdout, self.height)?;

        crossterm::execute!(&mut self.stdout)?;

        Ok(())
    }

    pub fn close(&mut self) -> Result<(), Error> {
        terminal::disable_raw_mode()?;
        crossterm::execute!(self.stdout, LeaveAlternateScreen)?;

        Ok(())
    }
}

pub enum Event {
    Input(String),
    Quit,
}
