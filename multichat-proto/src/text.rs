use serde::{Deserialize, Serialize};
use std::borrow::Cow;

/// A chunk of text with some style applied to it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Chunk<'a> {
    /// Text contents of this chunk.
    pub contents: Cow<'a, str>,
    /// Chunk style.
    pub style: Style,
}

/// Style of a message chunk.
#[derive(Serialize, Deserialize, Default, Clone, Copy, Debug)]
pub struct Style {
    /// Bold text.
    pub bold: bool,
    /// Italic text.
    pub italic: bool,
    /// RGB color, if any.
    pub color: Option<(u8, u8, u8)>,
}
