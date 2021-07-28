mod args;

use std::borrow::Cow;

pub enum Command<'a> {
    Connect { server: Cow<'a, str> },
    Disconnect,
    Groups,
    Users,
}
