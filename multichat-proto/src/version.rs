use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

/// Current protocol version - 1.0.
pub const VERSION: Version = Version { major: 1, minor: 0 };

/// Protocol version, sent by server as the first message when a connection is established.
#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

impl Version {
    /// Checks if `self` is compatible with the other version.
    ///
    /// Versions are compatible only if major versions are equal.
    pub fn is_compatible(&self, other: Version) -> bool {
        self.major == other.major
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}
