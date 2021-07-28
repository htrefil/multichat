use std::borrow::Cow;

struct Args<'a> {
    data: &'a str,
    offset: usize,
}

impl<'a> Iterator for Args<'a> {
    type Item = Result<Cow<'a, str>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        #[derive(Eq, PartialEq)]
        enum State {
            Borrowed(usize, usize),
            Owned(String),
            Empty,
        }

        let Self { ref data, offset } = self;

        let mut quote = false;
        let mut escape = false;
        let mut state = State::Empty;

        for c in data[*offset..].chars() {
            let start = *offset;
            let bytes = c.len_utf8();

            *offset += bytes;

            if c.is_ascii_whitespace() {
                if escape {
                    return Some(Err(Error::UnknownEscape(c)));
                }

                match &mut state {
                    State::Borrowed(_, _) | State::Owned(_) if !quote => break,
                    State::Borrowed(_, length) => *length += bytes,
                    State::Owned(data) => data.push(c),
                    State::Empty => {}
                }

                continue;
            }

            if escape {
                escape = false;

                let c = match c {
                    'r' => '\r',
                    'n' => '\n',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    _ => return Some(Err(Error::UnknownEscape(c))),
                };

                match &mut state {
                    State::Borrowed(offset, length) => {
                        let mut data = data[*offset..*offset + *length].to_owned();
                        data.push(c);

                        state = State::Owned(data);
                    }
                    State::Owned(data) => data.push(c),
                    State::Empty => state = State::Owned(c.to_string()),
                }

                continue;
            }

            if c == '"' {
                if quote {
                    quote = false;
                    break;
                }

                if state != State::Empty {
                    return Some(Err(Error::UnexpectedQuote));
                }

                quote = true;
                state = State::Borrowed(*offset, 0);
                continue;
            }

            if c == '\\' {
                escape = true;
                continue;
            }

            match &mut state {
                State::Borrowed(_, length) => *length += bytes,
                State::Owned(data) => data.push(c),
                State::Empty => state = State::Borrowed(start, bytes),
            }
        }

        if quote {
            return Some(Err(Error::UnfinishedQuote));
        }

        if escape {
            return Some(Err(Error::UnfinishedEscape));
        }

        match state {
            State::Borrowed(offset, length) => {
                Some(Cow::Borrowed(&self.data[offset..offset + length]))
            }
            State::Owned(data) => Some(Cow::Owned(data)),
            State::Empty => None,
        }
        .map(Ok)
    }
}

#[derive(Debug)]
pub enum Error {
    UnexpectedQuote,
    UnfinishedEscape,
    UnfinishedQuote,
    UnknownEscape(char),
}

pub fn args(data: &str) -> impl Iterator<Item = Result<Cow<'_, str>, Error>> {
    Args { data, offset: 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(input: &str, output: &[&str]) {
        let input = args(input).map(|item| item.unwrap()).collect::<Vec<_>>();

        assert_eq!(&input, output);
    }

    #[test]
    fn words() {
        check("  hello, world", &["hello,", "world"]);
        check("", &[]);
    }

    #[test]
    fn quotes() {
        check(r#"hello "world""#, &["hello", "world"]);
        check(r#"hello "nice world""#, &["hello", "nice world"]);
        check(r#""""#, &[""]);
    }

    #[test]
    fn escapes() {
        check(r#"\r \n \""#, &["\r", "\n", "\""]);
        check(r#""quoted \\\r\n\t\"""#, &["quoted \\\r\n\t\""]);
    }

    #[test]
    #[should_panic]
    fn unexpected_quote() {
        check(r#"abcd"hello""#, &[]);
    }
}
