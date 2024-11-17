use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
pub struct Config {
    pub telegram: Telegram,
    pub multichat: Multichat,
    pub chats: Vec<Chat>,
}

#[derive(Deserialize)]
pub struct Telegram {
    pub token: String,
}

#[derive(Deserialize)]
pub struct Multichat {
    pub server: String,
    pub certificate: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Chat {
    pub multichat_group: String,
    pub telegram_chat: i64,
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
