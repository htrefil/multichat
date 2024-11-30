use multichat_client::{MaybeTlsClient, Update, UpdateKind};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::{io, mem, slice};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{
    ChatAction, ChatId, InputFile, InputMedia, InputMediaAudio, InputMediaDocument,
    InputMediaPhoto, InputMediaVideo, ParseMode, UserId,
};
use teloxide::{Bot, RequestError};
use thiserror::Error;
use tokio::sync::mpsc::Receiver;
use tokio::time;

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
    let mut telegram_users = HashMap::<(UserId, ChatId), TelegramUser>::new();
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

                    let entry = telegram_users.entry((event.user_id, event.chat_id));
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
                                let uid = client.init_user(*gid, &user_name).await?;

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
                    let user = match telegram_users.remove(&(event.user_id, event.chat_id)) {
                        Some(user) => user,
                        None => continue,
                    };

                    for (gid, uid) in user.gid_uid {
                        client.destroy_user(gid, uid).await?;
                    }
                }
            },
            Event::Multichat(Update {
                kind: UpdateKind::InitGroup { .. } | UpdateKind::DestroyGroup { .. },
                ..
            }) => continue,
            Event::Multichat(update) => {
                let chat_ids = group_to_chat.get(&update.gid).unwrap();

                match update.kind {
                    UpdateKind::InitUser { uid, name } => {
                        let owned = owned.remove(&(update.gid, uid));
                        let user =
                            multichat_users
                                .entry((update.gid, uid))
                                .or_insert(MultichatUser {
                                    name,
                                    typing: false,
                                    owned,
                                });

                        if user.owned {
                            continue;
                        }

                        let message = format!("*{}*: joined", user.name.markdown_safe());

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_message(*chat_id, &message)
                                    .parse_mode(ParseMode::MarkdownV2)
                                    .disable_notification(true)
                                    .await
                            })
                            .await?;
                        }

                        let typing = multichat_users
                            .iter()
                            .any(|((ugid, _), user)| *ugid == update.gid && user.typing);

                        if !typing {
                            continue;
                        }

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_chat_action(*chat_id, ChatAction::Typing).await
                            })
                            .await?;
                        }
                    }
                    UpdateKind::DestroyUser { uid } => {
                        let user = multichat_users.remove(&(update.gid, uid)).unwrap();
                        if user.owned {
                            continue;
                        }

                        let message = format!("*{}*: left", user.name.markdown_safe());

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_message(*chat_id, &message)
                                    .parse_mode(ParseMode::MarkdownV2)
                                    .disable_notification(true)
                                    .await
                            })
                            .await?;
                        }

                        let typing = multichat_users
                            .iter()
                            .any(|((ugid, _), user)| *ugid == update.gid && user.typing);

                        if !typing {
                            continue;
                        }

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_chat_action(*chat_id, ChatAction::Typing).await
                            })
                            .await?;
                        }
                    }
                    UpdateKind::Message { uid, message } => {
                        let user = multichat_users.get(&(update.gid, uid)).unwrap();
                        if user.owned {
                            for attachment in message.attachments {
                                client.ignore_attachment(attachment.id).await?;
                            }

                            continue;
                        }

                        let text = format!(
                            "*{}*: {}",
                            user.name.markdown_safe(),
                            message.text.markdown_safe()
                        );

                        if !message.attachments.is_empty() {
                            let mut attachments = Vec::with_capacity(message.attachments.len());

                            for attachment in message.attachments {
                                if attachment.size > 50 * 1024 * 1024 {
                                    tracing::warn!(id = %attachment.id, "Attachment is too large, ignoring");
                                    continue;
                                }

                                attachments.push(client.download_attachment(attachment.id).await?);
                            }

                            // Split the attachments into chunks of 10, which is the maximum allowed by Telegram.
                            let len = attachments.len();
                            let chat_ids = group_to_chat.get(&update.gid).unwrap();

                            let mut media_group = Vec::new();
                            for (i, attachment) in attachments.into_iter().enumerate() {
                                let text = if media_group.is_empty() {
                                    Some(text.clone())
                                } else {
                                    None
                                };

                                media_group.push(into_input_media(attachment, text));

                                if media_group.len() == 10 || i == len - 1 {
                                    for chat_id in chat_ids {
                                        rate_limit(|| async {
                                            bot.send_media_group(*chat_id, media_group.clone())
                                                .await
                                        })
                                        .await?;
                                    }

                                    media_group.clear();
                                }
                            }
                        } else {
                            for chat_id in chat_ids {
                                rate_limit(|| async {
                                    bot.send_message(*chat_id, &text)
                                        .parse_mode(ParseMode::MarkdownV2)
                                        .await
                                })
                                .await?;
                            }
                        }

                        let typing = multichat_users
                            .iter()
                            .any(|((ugid, _), user)| *ugid == update.gid && user.typing);

                        if !typing {
                            continue;
                        }

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_chat_action(*chat_id, ChatAction::Typing).await
                            })
                            .await?;
                        }
                    }
                    UpdateKind::Rename {
                        uid,
                        name: new_name,
                    } => {
                        let user = multichat_users.get_mut(&(update.gid, uid)).unwrap();
                        let old_name = mem::replace(&mut user.name, new_name.clone());

                        if user.owned {
                            continue;
                        }

                        let message = format!(
                            "*{}*: renamed to *{}*",
                            old_name.markdown_safe(),
                            new_name.markdown_safe()
                        );

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_message(*chat_id, &message)
                                    .parse_mode(ParseMode::MarkdownV2)
                                    .disable_notification(true)
                                    .await
                            })
                            .await?;
                        }

                        let typing = multichat_users
                            .iter()
                            .any(|((ugid, _), user)| *ugid == update.gid && user.typing);

                        if !typing {
                            continue;
                        }

                        for chat_id in chat_ids {
                            rate_limit(|| async {
                                bot.send_chat_action(*chat_id, ChatAction::Typing).await
                            })
                            .await?;
                        }
                    }
                    UpdateKind::StartTyping { uid } => {
                        let user = multichat_users.get_mut(&(update.gid, uid)).unwrap();
                        user.typing = true;

                        for chat_id in &group_to_chat[&update.gid] {
                            rate_limit(|| async {
                                bot.send_chat_action(*chat_id, ChatAction::Typing).await
                            })
                            .await?;
                        }

                        continue;
                    }
                    UpdateKind::StopTyping { uid } => {
                        let user = multichat_users.get_mut(&(update.gid, uid)).unwrap();
                        user.typing = false;

                        let typing = multichat_users
                            .iter()
                            .any(|((ugid, _), user)| *ugid == update.gid && user.typing);

                        if typing {
                            continue;
                        }

                        for chat_id in &group_to_chat[&update.gid] {
                            rate_limit(|| async {
                                bot.send_chat_action(*chat_id, ChatAction::Typing).await
                            })
                            .await?;
                        }

                        continue;
                    }
                    UpdateKind::InitGroup { .. } | UpdateKind::DestroyGroup { .. } => {
                        // Handled above.
                        unreachable!()
                    }
                }
            }
        }
    }

    Ok(())
}

fn into_input_media(data: Vec<u8>, caption: Option<String>) -> InputMedia {
    // Match on the first bytes to determine if it's a photo, video, or a generic document.
    match &data[..] {
        // Photo.
        [0xFF, 0xD8, 0xFF, ..] | [0x89, b'P', b'N', b'G', ..] | [0x52, 0x49, 0x46, 0x46, ..] => {
            let file = InputFile::memory(data);

            let mut media = InputMediaPhoto::new(file).parse_mode(ParseMode::MarkdownV2);
            media.caption = caption;

            InputMedia::Photo(media)
        }
        // Video.
        [0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', ..] => {
            let file = InputFile::memory(data);

            let mut media = InputMediaVideo::new(file).parse_mode(ParseMode::MarkdownV2);
            media.caption = caption;

            InputMedia::Video(media)
        }
        // Audio.
        [0x49, 0x44, 0x33, 0x03, ..] | [0xFF, 0xF1, ..] | [0xFF, 0xF9, ..] => {
            let file = InputFile::memory(data);

            let mut media = InputMediaAudio::new(file).parse_mode(ParseMode::MarkdownV2);
            media.caption = caption;

            InputMedia::Audio(media)
        }
        // Document.
        _ => {
            let file = InputFile::memory(data);

            let mut media = InputMediaDocument::new(file).parse_mode(ParseMode::MarkdownV2);
            media.caption = caption;

            InputMedia::Document(media)
        }
    }
}

async fn rate_limit<T, C: Fn() -> F, F: Future<Output = Result<T, RequestError>>>(
    c: C,
) -> Result<T, RequestError> {
    loop {
        match c().await {
            Ok(result) => return Ok(result),
            Err(RequestError::RetryAfter(duration)) => {
                let duration = duration.duration();
                tracing::warn!(?duration, "Rate limited, waiting");

                time::sleep(duration).await;
                continue;
            }
            Err(err) => return Err(err),
        }
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
    typing: bool,
}
