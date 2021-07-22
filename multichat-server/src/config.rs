use serde::Deserialize;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::num::NonZeroUsize;
use std::path::PathBuf;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    pub listen_addr: SocketAddr,
    pub tls: Option<Tls>,
    pub update_buffer: Option<NonZeroUsize>,
    pub groups: HashSet<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Tls {
    pub identity_path: PathBuf,
    #[serde(default)]
    pub identity_password: String,
}
