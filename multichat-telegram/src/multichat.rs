use multichat_client::{MaybeTlsClient, Update, UpdateKind};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::{io, mem, slice};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{
    ChatId, InputFile, InputMedia, InputMediaAudio, InputMediaDocument, InputMediaPhoto,
    InputMediaVideo, ParseMode, UserId,
};
use teloxide::{Bot, RequestError};
use thiserror::Error;
use tokio::sync::mpsc::Receiver;

use crate::markdown_safe::MarkdownSafeExt;
use crate::telegram::{Event as TelegramEvent, EventKind};

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Request(#[from] RequestError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub async fn run(
    mut client: MaybeTlsClient,
    bot: Bot,
    chat_to_group: &HashMap<ChatId, HashSet<u32>>,
    group_to_chat: &HashMap<u32, HashSet<ChatId>>,
    mut receiver: Receiver<TelegramEvent>,
) -> Result<(), Error> {
    for (gid, _) in group_to_chat {
        client.join_group(*gid).await?;
    }

    let mut telegram_users = HashMap::<UserId, TelegramUser>::new();
    let mut multichat_users = HashMap::new();

    let mut owned = HashSet::new();

    loop {
        let event = tokio::select! {
            event = receiver.recv() => match event {
                Some(event) => Event::Telegram(event),
                None => break,
            },
            update = client.read_update() => Event::Multichat(update?),
        };

        match event {
            Event::Telegram(event) => match event.kind {
                EventKind::Message {
                    user_name,
                    text,
                    attachment,
                } => {
                    let chat_id = event.chat_id;

                    let gids = match chat_to_group.get(&chat_id) {
                        Some(gids) => gids,
                        None => {
                            tracing::warn!(%chat_id, "Telegram chat not found");
                            continue;
                        }
                    };

                    let entry = telegram_users.entry(event.user_id);
                    let user = match entry {
                        Entry::Occupied(entry) => {
                            let user = entry.into_mut();
                            if user.name != user_name {
                                for (gid, uid) in &user.gid_uid {
                                    client.rename_user(*gid, *uid, &user_name).await?;
                                }

                                user.name = user_name;
                            }

                            user
                        }
                        Entry::Vacant(_) => {
                            let mut gid_uid = Vec::new();

                            for gid in gids {
                                let uid = client.join_user(*gid, &user_name).await?;

                                gid_uid.push((*gid, uid));
                                owned.insert((*gid, uid));
                            }

                            entry.or_insert(TelegramUser {
                                name: user_name,
                                gid_uid,
                            })
                        }
                    };

                    let attachment = attachment.map(|data| Cow::Owned(data));

                    let attachments = match &attachment {
                        Some(attachment) => slice::from_ref(attachment),
                        None => &[],
                    };

                    for (gid, uid) in &user.gid_uid {
                        tracing::debug!(%gid, %uid, msg = ?text, "Sending message to Multichat");
                        client.send_message(*gid, *uid, &text, attachments).await?;
                    }
                }
                EventKind::Leave => {
                    let user = match telegram_users.remove(&event.user_id) {
                        Some(user) => user,
                        None => continue,
                    };

                    for (gid, uid) in user.gid_uid {
                        client.leave_user(gid, uid).await?;
                    }
                }
            },
            Event::Multichat(update) => {
                let message = match update.kind {
                    UpdateKind::Join(name) => {
                        let owned = owned.remove(&(update.gid, update.uid));
                        let user = multichat_users
                            .entry((update.gid, update.uid))
                            .or_insert(MultichatUser { name, owned });

                        if user.owned {
                            continue;
                        }

                        format!("*{}*: joined", user.name.markdown_safe())
                    }
                    UpdateKind::Leave => {
                        let user = multichat_users.remove(&(update.gid, update.uid)).unwrap();
                        if user.owned {
                            continue;
                        }

                        format!("*{}*: left", user.name.markdown_safe())
                    }
                    UpdateKind::Message(message) => {
                        let user = multichat_users.get(&(update.gid, update.uid)).unwrap();
                        if user.owned {
                            for attachment in message.attachments {
                                client.ignore_attachment(attachment.id).await?;
                            }

                            continue;
                        }

                        let text = format!(
                            "*{}*: {}",
                            user.name.markdown_safe(),
                            message.message.markdown_safe()
                        );

                        if !message.attachments.is_empty() {
                            let mut attachments = Vec::new();

                            for attachment in message.attachments {
                                let data = client.download_attachment(attachment.id).await?;
                                attachments.push(data);
                            }

                            for chat_id in group_to_chat.get(&update.gid).unwrap() {
                                bot.send_media_group(
                                    *chat_id,
                                    attachments.iter().map(|attachment| {
                                        into_input_media(attachment.clone(), text.clone())
                                    }),
                                )
                                .await?;
                            }

                            continue;
                        }

                        text
                    }
                    UpdateKind::Rename(new_name) => {
                        let user = multichat_users.get_mut(&(update.gid, update.uid)).unwrap();
                        let old_name = mem::replace(&mut user.name, new_name.clone());

                        if user.owned {
                            continue;
                        }

                        format!(
                            "*{}*: renamed to *{}*",
                            old_name.markdown_safe(),
                            new_name.markdown_safe()
                        )
                    }
                };

                let chat_ids = group_to_chat.get(&update.gid).unwrap();
                for chat_id in chat_ids {
                    tracing::debug!(%chat_id, msg = ?message, "Sending message to Telegram");

                    bot.send_message(*chat_id, &message)
                        .parse_mode(ParseMode::MarkdownV2)
                        .await?;
                }
            }
        }
    }

    Ok(())
}

fn into_input_media(data: Vec<u8>, caption: String) -> InputMedia {
    // Match on the first bytes to determine if it's a photo, video, or a generic document.
    match &data[..] {
        // Photo.
        [0xFF, 0xD8, 0xFF, ..] | [0x89, b'P', b'N', b'G', ..] | [0x52, 0x49, 0x46, 0x46, ..] => {
            InputMedia::Photo(
                InputMediaPhoto::new(InputFile::memory(data))
                    .caption(caption)
                    .parse_mode(ParseMode::MarkdownV2),
            )
        }
        // Video.
        [0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', ..] => InputMedia::Video(
            InputMediaVideo::new(InputFile::memory(data))
                .caption(caption)
                .parse_mode(ParseMode::MarkdownV2),
        ),
        // Audio.
        [0x49, 0x44, 0x33, 0x03, ..] | [0xFF, 0xF1, ..] | [0xFF, 0xF9, ..] => InputMedia::Audio(
            InputMediaAudio::new(InputFile::memory(data))
                .caption(caption)
                .parse_mode(ParseMode::MarkdownV2),
        ),
        // Document.
        _ => InputMedia::Document(
            InputMediaDocument::new(InputFile::memory(data))
                .caption(caption)
                .parse_mode(ParseMode::MarkdownV2),
        ),
    }
}

enum Event {
    Telegram(TelegramEvent),
    Multichat(Update),
}

struct TelegramUser {
    name: String,
    gid_uid: Vec<(u32, u32)>,
}

struct MultichatUser {
    name: String,
    owned: bool,
}
