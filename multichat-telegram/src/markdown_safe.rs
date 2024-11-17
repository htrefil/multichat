use std::fmt::{self, Display, Formatter};

pub struct MarkdownSafe<T>(pub T);

impl<T: AsRef<str>> Display for MarkdownSafe<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut last_char = ' ';
        for c in self.0.as_ref().chars() {
            match c {
                '*' | '_' | '[' | ']' | '(' | ')' | '~' | '`' | '>' | '#' | '+' | '-' | '='
                | '|' | '{' | '}' | '.' | '!'
                    if last_char != '\\' =>
                {
                    write!(f, "\\{}", c)?
                }
                _ => write!(f, "{}", c)?,
            }
            last_char = c;
        }

        Ok(())
    }
}

pub trait MarkdownSafeExt: AsRef<str> {
    fn markdown_safe(&self) -> MarkdownSafe<&Self> {
        MarkdownSafe(self)
    }
}

impl<T: AsRef<str>> MarkdownSafeExt for T {}
