use multichat_proto::Chunk;
use std::slice;

/// Helper trait to make sending messages from a client more ergonomic.
pub trait AsChunks<'a> {
    type Output: AsRef<[Chunk<'a>]>;

    fn as_chunks(self) -> Self::Output;
}

impl<'a> AsChunks<'a> for &'a Chunk<'a> {
    type Output = &'a [Chunk<'a>];

    fn as_chunks(self) -> Self::Output {
        slice::from_ref(self)
    }
}

impl<'a> AsChunks<'a> for &'a Vec<Chunk<'a>> {
    type Output = &'a Vec<Chunk<'a>>;

    fn as_chunks(self) -> Self::Output {
        self
    }
}

impl<'a> AsChunks<'a> for Vec<Chunk<'a>> {
    type Output = Vec<Chunk<'a>>;

    fn as_chunks(self) -> Self::Output {
        self
    }
}

impl<'a> AsChunks<'a> for &'a str {
    type Output = SingleChunk<'a>;

    fn as_chunks(self) -> Self::Output {
        SingleChunk(Chunk {
            contents: self.into(),
            style: Default::default(),
        })
    }
}

impl AsChunks<'static> for String {
    type Output = SingleChunk<'static>;

    fn as_chunks(self) -> Self::Output {
        SingleChunk(Chunk {
            contents: self.into(),
            style: Default::default(),
        })
    }
}

pub struct SingleChunk<'a>(Chunk<'a>);

impl<'a> AsRef<[Chunk<'a>]> for SingleChunk<'a> {
    fn as_ref(&self) -> &[Chunk<'a>] {
        slice::from_ref(&self.0)
    }
}
