use serde::de::{Deserialize, Deserializer, Error, Visitor};
use serde::ser::{Serialize, Serializer};
use std::fmt::{self, Debug, Display, Formatter, Write};
use std::str::FromStr;
use thiserror::Error;

const LENGTH: usize = 32;

/// An access token used to authenticate clients.
/// The token is a 256-bit randomly generated value encoded as a hexadecimal string.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AccessToken([u8; LENGTH]);

impl FromStr for AccessToken {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 2 * LENGTH {
            return Err(ParseError);
        }

        let mut result = [0; LENGTH];
        for (i, b) in result.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..][..2], 16).map_err(|_| ParseError)?;
        }

        Ok(Self(result))
    }
}

impl<'de> Deserialize<'de> for AccessToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AccessTokenVisitor;

        impl Visitor<'_> for AccessTokenVisitor {
            type Value = AccessToken;

            fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
                write!(formatter, "an access token")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                AccessToken::from_str(value).map_err(E::custom)
            }

            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: Error,
            {
                if value.len() != LENGTH {
                    return Err(E::invalid_length(value.len(), &self));
                }

                let mut bytes = [0; LENGTH];
                bytes.copy_from_slice(value);

                Ok(AccessToken(bytes))
            }
        }

        if deserializer.is_human_readable() {
            deserializer.deserialize_str(AccessTokenVisitor)
        } else {
            deserializer.deserialize_bytes(AccessTokenVisitor)
        }
    }
}

impl Serialize for AccessToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if serializer.is_human_readable() {
            let mut buffer = String::with_capacity(2 * LENGTH);
            write!(buffer, "{}", self).unwrap();

            serializer.serialize_str(&buffer)
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

impl Display for AccessToken {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }

        Ok(())
    }
}

impl Debug for AccessToken {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        Display::fmt(self, f)
    }
}

#[derive(Debug, Error)]
#[error("Invalid access token")]
pub struct ParseError;
