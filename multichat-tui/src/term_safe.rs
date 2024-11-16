use std::fmt::Display;

use crossterm::style::{self, StyledContent, Stylize};

pub struct TermSafe<T>(T);

impl<T: AsRef<str>> Display for TermSafe<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for c in self.0.as_ref().chars() {
            if c.is_ascii_control() {
                write!(f, "\\u{:04X}", c as u32)?;
                continue;
            }

            write!(f, "{}", c)?;
        }

        Ok(())
    }
}

pub trait TermSafeExt: AsRef<str> {
    fn term_safe(&self) -> TermSafe<&Self> {
        TermSafe(self)
    }
}

impl<T: AsRef<str>> TermSafeExt for T {}

impl<T: AsRef<str>> Stylize for TermSafe<T> {
    type Styled = StyledContent<Self>;

    fn stylize(self) -> Self::Styled {
        style::style(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ansi_escape() {
        let input = "\x1b[31mHello, \x1b[32mworld!\x1b[0m";
        let expected = "\\u001B[31mHello, \\u001B[32mworld!\\u001B[0m";

        assert_eq!(input.term_safe().to_string(), expected);
    }

    #[test]
    fn newline() {
        let input = "Hello,\nworld!";
        let expected = "Hello,\\u000Aworld!";

        assert_eq!(input.term_safe().to_string(), expected);
    }
}
