use serde::de;
use serde::ser;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cmp::{Eq, PartialEq};
use std::fmt::{self, Formatter};
use std::slice;

/// A message as sent by an user.
#[derive(Clone, Debug)]
pub struct Message<'a>(Contents<'a>);

impl<'a> Message<'a> {
    /// Creates a new message consisting of specified chunks.
    pub fn new(chunks: impl Into<Cow<'a, [Chunk<'a>]>>) -> Self {
        Self(Contents::Variable(chunks.into()))
    }

    /// Creates an empty message.
    pub fn empty() -> Self {
        const EMPTY: &[Chunk<'_>] = &[];

        Message(Contents::Variable(EMPTY.into()))
    }

    /// Creates a message consisting of one unstyled chunk.
    pub fn plain(contents: impl Into<Cow<'a, str>>) -> Self {
        Self(Contents::Single(Chunk {
            contents: contents.into(),
            style: Style::default(),
        }))
    }

    /// Returns the chunks of this message.
    pub fn chunks(&self) -> &[Chunk<'a>] {
        match &self.0 {
            Contents::Single(chunk) => slice::from_ref(chunk),
            Contents::Variable(chunks) => &*chunks,
        }
    }

    /// Returns the text of this message.
    ///
    /// If the message consists of multiple chunks, they will be concatenated in a [`String`].
    ///
    /// Otherwise, a reference to the first (and only) chunk's contents is returned.
    pub fn text(&self) -> Cow<'_, str> {
        let chunks = self.chunks();

        match &chunks {
            [first] => first.contents.as_ref().into(),
            [_, ..] => chunks
                .iter()
                .map(|chunk| chunk.contents.as_ref())
                .collect::<String>()
                .into(),
            [] => "".into(),
        }
    }
}

impl<'a> Eq for Message<'a> {}

impl<'a, 'b> PartialEq<Message<'a>> for Message<'b> {
    fn eq(&self, other: &Message<'a>) -> bool {
        self.chunks() == other.chunks()
    }
}

// Implement Serialize and Deserialize manually due to our single value optimization.
impl<'a> ser::Serialize for Message<'a> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.chunks().serialize(serializer)
    }
}

impl<'de, 'a> de::Deserialize<'de> for Message<'a> {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor<'a> {
            _phantom: &'a (),
        }

        impl<'de, 'a> de::Visitor<'de> for Visitor<'a> {
            type Value = Message<'a>;

            fn expecting(&self, f: &mut Formatter) -> fmt::Result {
                write!(f, "an array of chunks")
            }

            fn visit_seq<S: de::SeqAccess<'de>>(self, mut seq: S) -> Result<Self::Value, S::Error> {
                let contents = match seq.next_element()? {
                    Some(first) => match seq.next_element()? {
                        Some(second) => {
                            let mut buffer = vec![first, second];
                            while let Some(element) = seq.next_element()? {
                                buffer.push(element);
                            }

                            Contents::Variable(buffer.into())
                        }
                        None => Contents::Single(first),
                    },
                    None => Contents::Variable(Vec::new().into()),
                };

                Ok(Message(contents))
            }
        }

        deserializer.deserialize_seq(Visitor { _phantom: &() })
    }
}

// Most messages will contain just one unstyled chunk, so it makes sense to avoid
// a heap allocation that comes with having a single-element Vec.
#[derive(Clone, Debug, Serialize, Deserialize)]
enum Contents<'a> {
    Single(Chunk<'a>),
    Variable(Cow<'a, [Chunk<'a>]>),
}

/// A chunk of text with some style applied to it.
#[derive(Serialize, Deserialize, Clone, Debug, Eq, PartialEq)]
pub struct Chunk<'a> {
    /// Text contents of this chunk.
    pub contents: Cow<'a, str>,
    /// Chunk style.
    pub style: Style,
}

/// Style of a message chunk.
#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug, Eq, PartialEq)]
pub struct Style {
    /// Bold text.
    pub bold: bool,
    /// Italic text.
    pub italic: bool,
    /// RGB color, if any.
    pub color: Option<(u8, u8, u8)>,
}
