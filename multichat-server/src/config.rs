use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashSet;
use std::fmt::{self, Formatter};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub listen: SocketAddr,
    pub tls: Option<Tls>,
    pub update_buffer: Option<NonZeroUsize>,
    #[serde(deserialize_with = "deserialize_size")]
    pub max_size: usize,
    pub groups: HashSet<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Tls {
    pub certificate: PathBuf,
    pub key: PathBuf,
}

fn deserialize_size<'de, D>(deserializer: D) -> Result<usize, D::Error>
where
    D: Deserializer<'de>,
{
    struct SizeVisitor;

    impl<'de> Visitor<'de> for SizeVisitor {
        type Value = usize;

        fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
            formatter.write_str("a size")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            let (size, unit) = value.split_once(char::is_whitespace).ok_or_else(|| {
                E::custom("size must be a number followed by a unit (e.g. 1 KiB)")
            })?;

            let size: usize = size.parse().map_err(E::custom)?;
            let mul = match unit {
                "B" => 1,
                "KiB" => 1024,
                "MiB" => 1024 * 1024,
                "GiB" => 1024 * 1024 * 1024,
                _ => return Err(E::custom("unknown unit")),
            };

            let size = size
                .checked_mul(mul)
                .ok_or_else(|| E::custom("size is too large"))?;

            Ok(size)
        }
    }

    deserializer.deserialize_str(SizeVisitor)
}
