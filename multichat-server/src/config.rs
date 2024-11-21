use multichat_proto::AccessToken;
use serde::de::{Error, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::HashSet;
use std::fmt::{self, Formatter};
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub listen: SocketAddr,
    pub tls: Option<Tls>,
    pub update_buffer: Option<NonZeroUsize>,
    #[serde(deserialize_with = "deserialize_size")]
    pub max_size: usize,
    #[serde(default, deserialize_with = "deserialize_duration")]
    pub ping_interval: Option<Duration>,
    #[serde(default, deserialize_with = "deserialize_duration")]
    pub ping_timeout: Option<Duration>,
    pub groups: HashSet<String>,
    pub access_tokens: HashSet<AccessToken>,
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

fn deserialize_duration<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    struct DurationVisitor;

    impl Visitor<'_> for DurationVisitor {
        type Value = Option<Duration>;

        fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
            write!(formatter, "a duration of time")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: Error,
        {
            humantime::Duration::from_str(v)
                .map_err(E::custom)
                .map(Into::into)
                .map(Some)
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: Error,
        {
            Ok(None)
        }
    }

    deserializer.deserialize_str(DurationVisitor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_parses() {
        let config = include_str!("../example/config.toml");
        toml::from_str::<Config>(config).unwrap();
    }
}
