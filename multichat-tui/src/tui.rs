use crate::command::{Command, Error as CommandError};
use crate::screen::{Event as ScreenEvent, Level, Screen};
use crate::term_safe::TermSafeExt;

use crossterm::style::Stylize;
use multichat_client::proto::ServerInit;
use multichat_client::{BasicClient, BasicConnectError, ClientBuilder, Update, UpdateKind};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::io::Error;
use std::{future, mem};
use tokio::sync::mpsc;

pub async fn run(screen: &mut Screen) -> Result<(), Error> {
    screen.log(
        Level::Info,
        format!(
            "Multichat TUI v{}, using protocol v{}",
            env!("CARGO_PKG_VERSION"),
            multichat_client::proto::VERSION
        ),
    );

    let mut connecting = false;
    let mut state = None::<State>;
    let (sender, mut receiver) = mpsc::channel(1);

    loop {
        screen.render()?;

        let update = async {
            match &mut state {
                Some(state) => state.client.read_update().await,
                None => future::pending().await,
            }
        };

        let event = tokio::select! {
            update = update => Event::Update(update),
            event = screen.process() => {
                match event? {
                    Some(event) => Event::Screen(event),
                    None => continue,
                }
            },
            event = receiver.recv() => Event::Connect(event.unwrap()),
        };

        match event {
            Event::Screen(event) => match event {
                ScreenEvent::Input(input) => {
                    let command = match Command::try_from(&*input) {
                        Ok(command) => command,
                        Err(CommandError::NotACommand) => {
                            if let Some(state) = &mut state {
                                if let Some((gid, uid)) = state.current {
                                    state.client.send_message(gid, uid, &input, &[]).await?;
                                } else {
                                    screen.log(Level::Error, "No active user");
                                }
                            }

                            continue;
                        }
                        Err(err) => {
                            screen.log(Level::Error, format!("{}", err));
                            continue;
                        }
                    };

                    match command {
                        Command::Connect { server } => {
                            if connecting {
                                screen.log(Level::Error, "Already connecting");
                                continue;
                            }

                            state = None;
                            connecting = true;

                            let server = server.into_owned();
                            let sender = sender.clone();

                            screen.log(Level::Info, "Attempting to connect to server");

                            tokio::spawn(async move {
                                let builder = ClientBuilder::basic();

                                tokio::select! {
                                    result = builder.connect(&*server) => {
                                        let _ = sender.send(result).await;
                                    }
                                    _ = sender.closed() => {}
                                }
                            });

                            continue;
                        }
                        Command::Groups => {
                            let state = match &state {
                                Some(state) => state,
                                None => {
                                    screen.log(Level::Error, "Not connected to server");
                                    continue;
                                }
                            };

                            for (gid, group) in &state.groups {
                                screen.log(Level::Info, format!("* {} ({})", group.name, gid));
                            }
                        }
                        Command::Users => {
                            let state = match &state {
                                Some(state) => state,
                                None => {
                                    screen.log(Level::Error, "Not connected to server");
                                    continue;
                                }
                            };

                            for (gid, group) in &state.groups {
                                screen.log(
                                    Level::Info,
                                    format!("* {} ({})", group.name.term_safe(), gid),
                                );

                                for (uid, user) in &group.users {
                                    screen.log(
                                        Level::Info,
                                        format!("  * {} ({})", user.name.term_safe(), uid),
                                    );
                                }
                            }
                        }
                        Command::Disconnect => {
                            state = None;
                            connecting = false;
                        }
                        Command::Join { group, user } => {
                            let state = match state.as_mut() {
                                Some(state) => state,
                                None => {
                                    screen.log(Level::Error, "Not connected to server");
                                    continue;
                                }
                            };

                            let (gid, group) =
                                match state.groups.iter_mut().find(|(_, g)| group == g.name) {
                                    Some((gid, group)) => (*gid, group),
                                    None => {
                                        screen.log(Level::Error, "Unknown group");
                                        continue;
                                    }
                                };

                            if !group.joined {
                                state.client.join_group(gid).await?;
                                group.joined = true;

                                screen.log(
                                    Level::Info,
                                    format!("Joined group {}", group.name.term_safe()),
                                );
                            }

                            if let Some(user) = user {
                                let uid = state.client.join_user(gid, &*user).await?;
                                group.owned.insert(uid);
                            }
                        }
                        Command::Leave { group, uid } => {
                            let state = match state.as_mut() {
                                Some(state) => state,
                                None => {
                                    screen.log(Level::Error, "Not connected to server");
                                    continue;
                                }
                            };

                            let (gid, group) =
                                match state.groups.iter_mut().find(|(_, g)| group == g.name) {
                                    Some((gid, group)) => (*gid, group),
                                    None => {
                                        screen.log(Level::Error, "Unknown group");
                                        continue;
                                    }
                                };

                            if let Some(uid) = uid {
                                let user = match group.users.get(&uid) {
                                    Some(user) => user,
                                    None => {
                                        screen.log(Level::Error, "Unknown user");
                                        continue;
                                    }
                                };

                                if !user.owned {
                                    screen.log(Level::Error, "Cannot leave foreign user");
                                    continue;
                                }

                                state.client.leave_user(gid, uid).await?;
                            }
                        }
                        Command::Rename { group, uid, name } => {
                            let state = match state.as_mut() {
                                Some(state) => state,
                                None => {
                                    screen.log(Level::Error, "Not connected to server");
                                    continue;
                                }
                            };

                            let (gid, group) =
                                match state.groups.iter_mut().find(|(_, g)| group == g.name) {
                                    Some((gid, group)) => (*gid, group),
                                    None => {
                                        screen.log(Level::Error, "Unknown group");
                                        continue;
                                    }
                                };

                            let user = match group.users.get(&uid) {
                                Some(user) => user,
                                None => {
                                    screen.log(Level::Error, "Unknown user");
                                    continue;
                                }
                            };

                            if !user.owned {
                                screen.log(Level::Error, "Cannot rename foreign user");
                                continue;
                            }

                            state.client.rename_user(gid, uid, &*name).await?;
                        }
                        Command::Switch { group, uid } => {
                            let state = match state.as_mut() {
                                Some(state) => state,
                                None => {
                                    screen.log(Level::Error, "Not connected to server");
                                    continue;
                                }
                            };

                            let (gid, group) =
                                match state.groups.iter().find(|(_, g)| group == g.name) {
                                    Some((gid, group)) => (*gid, group),
                                    None => {
                                        screen.log(Level::Error, "Unknown group");
                                        continue;
                                    }
                                };

                            let user = match group.users.get(&uid) {
                                Some(user) => user,
                                None => {
                                    screen.log(Level::Error, "Unknown user");
                                    continue;
                                }
                            };

                            if !user.owned {
                                screen.log(Level::Error, "Cannot switch to foreign user");
                                continue;
                            }

                            state.current = Some((gid, uid));
                        }
                    }
                }
                ScreenEvent::Quit => return Ok(()),
            },
            Event::Connect(result) => {
                connecting = false;

                match result {
                    Ok((init, client)) => {
                        screen.log(Level::Info, "Connected to server");

                        let groups = init
                            .groups
                            .into_iter()
                            .map(|(name, gid)| {
                                (
                                    gid,
                                    Group {
                                        name,
                                        users: HashMap::new(),
                                        joined: false,
                                        owned: HashSet::new(),
                                    },
                                )
                            })
                            .collect();

                        state = Some(State {
                            groups,
                            client,
                            current: None,
                        });
                    }
                    Err(err) => {
                        screen.log(Level::Error, format!("Error connecting to server: {}", err));
                    }
                }
            }
            Event::Update(update) => {
                let update = match update {
                    Ok(update) => update,
                    Err(err) => {
                        screen.log(Level::Error, format!("Disconnected: {}", err));
                        state = None;
                        continue;
                    }
                };

                let state = state.as_mut().unwrap();
                let group = state.groups.get_mut(&update.gid).unwrap();

                match update.kind {
                    UpdateKind::Join(name) => {
                        screen.log(
                            Level::Info,
                            format!(
                                "[{}] {} ({}): joined",
                                group.name.term_safe(),
                                name.term_safe().bold(),
                                update.uid
                            ),
                        );

                        let owned = group.owned.remove(&update.uid);
                        if owned && state.current.is_none() {
                            state.current = Some((update.gid, update.uid));
                        }

                        group.users.insert(update.uid, User { name, owned });
                    }
                    UpdateKind::Leave => {
                        let name = group.users.remove(&update.uid).unwrap().name;

                        screen.log(
                            Level::Info,
                            format!(
                                "[{}] {} ({}): left",
                                group.name.term_safe(),
                                name.term_safe().bold(),
                                update.uid
                            ),
                        );
                    }
                    UpdateKind::Rename(name) => {
                        let old_name = mem::replace(
                            &mut group.users.get_mut(&update.uid).unwrap().name,
                            name.clone(),
                        );

                        screen.log(
                            Level::Info,
                            format!(
                                "[{}] {} ({}): renamed to {}",
                                group.name.term_safe(),
                                old_name.term_safe().bold(),
                                update.uid,
                                name.term_safe().bold()
                            ),
                        );
                    }
                    UpdateKind::Message(message) => {
                        let user = &group.users.get(&update.uid).unwrap().name;

                        screen.log(
                            Level::Info,
                            format!(
                                "[{}] {} ({}): {}",
                                group.name.term_safe(),
                                user.term_safe().bold(),
                                update.uid,
                                message.message.term_safe()
                            ),
                        );

                        for attachment in message.attachments {
                            screen.log(
                                Level::Info,
                                format!(
                                    "[{}] {} ({}): attachment {}, size {} b",
                                    group.name.term_safe(),
                                    user.term_safe().bold(),
                                    update.uid,
                                    attachment.id,
                                    attachment.size
                                ),
                            );

                            state.client.ignore_attachment(attachment.id).await?;
                        }
                    }
                }
            }
        }
    }
}

enum Event {
    Screen(ScreenEvent),
    Connect(Result<(ServerInit<'static>, BasicClient), BasicConnectError>),
    Update(Result<Update, Error>),
}

struct State {
    groups: HashMap<u32, Group>,
    client: BasicClient,
    current: Option<(u32, u32)>, // (gid, uid)
}

struct Group {
    name: Cow<'static, str>,
    users: HashMap<u32, User>,
    owned: HashSet<u32>,
    joined: bool,
}

struct User {
    name: String,
    owned: bool, // Did we create this user?
}
