mod args;

use std::borrow::Cow;
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug)]
pub enum Command<'a> {
    Connect {
        server: Cow<'a, str>,
    },
    Disconnect,
    Groups,
    Users,
    Join {
        group: Cow<'a, str>,
        user: Option<Cow<'a, str>>,
    },
    Leave {
        group: Cow<'a, str>,
        uid: Option<u32>,
    },
    Rename {
        group: Cow<'a, str>,
        uid: u32,
        name: Cow<'a, str>,
    },
    Switch {
        group: Cow<'a, str>,
        uid: u32,
    },
}

impl<'a> TryFrom<&'a str> for Command<'a> {
    type Error = Error;

    fn try_from(data: &'a str) -> Result<Self, Error> {
        let mut args = args::args(data);

        let command = args.next().ok_or(Error::NotACommand)??;
        let command = command
            .trim_start()
            .strip_prefix('/')
            .ok_or(Error::NotACommand)?;

        let command = match &*command {
            "connect" => Command::Connect {
                server: args.next().ok_or(Error::MissingArgument)??,
            },
            "disconnect" => Command::Disconnect,
            "groups" => Command::Groups,
            "users" => Command::Users,
            "join" => Command::Join {
                group: args.next().ok_or(Error::MissingArgument)??,
                user: args.next().transpose()?,
            },
            "leave" => Command::Leave {
                group: args.next().ok_or(Error::MissingArgument)??,
                uid: args
                    .next()
                    .transpose()?
                    .map(|user| user.parse().map_err(|_| Error::InvalidArgument))
                    .transpose()?,
            },
            "rename" => Command::Rename {
                group: args.next().ok_or(Error::MissingArgument)??,
                uid: args
                    .next()
                    .ok_or(Error::MissingArgument)??
                    .parse()
                    .map_err(|_| Error::InvalidArgument)?,
                name: args.next().ok_or(Error::MissingArgument)??,
            },
            "switch" => Command::Switch {
                group: args.next().ok_or(Error::MissingArgument)??,
                uid: args
                    .next()
                    .ok_or(Error::MissingArgument)??
                    .parse()
                    .map_err(|_| Error::InvalidArgument)?,
            },
            _ => return Err(Error::InvalidCommand),
        };

        if args.next().is_some() {
            return Err(Error::ExtraArgument);
        }

        Ok(command)
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("Invalid command")]
    InvalidCommand,
    #[error("Not a command")]
    NotACommand,
    #[error("Missing argument")]
    MissingArgument,
    #[error("Invalid argument")]
    InvalidArgument,
    #[error("Extra argument")]
    ExtraArgument,
    #[error(transparent)]
    Args(#[from] args::Error),
}
