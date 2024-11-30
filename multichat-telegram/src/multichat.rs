use multichat_client::{MaybeTlsClient, Update, UpdateKind};
use std::borrow::Cow;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::time::Duration;
use std::{io, mem, slice};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::Requester;
use teloxide::types::{
    ChatAction, ChatId, InputFile, InputMedia, InputMediaAudio, InputMediaDocument,
    InputMediaPhoto, InputMediaVideo, ParseMode, UserId,
};
use teloxide::{Bot, RequestError};
use thiserror::Error;
use tokio::sync::mpsc::{self, Receiver};
use tokio::task::JoinHandle;
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
    mut telegram_receiver: Receiver<TelegramEvent>,
) -> Result<(), Error> {
    let mut users = HashMap::<(UserId, ChatId), TelegramUser>::new();
    let mut groups = group_to_chat
        .keys()
        .map(|gid| {
            (
                *gid,
                Group {
                    users: HashMap::new(),
                    typing: None,
                },
            )
        })
        .collect::<HashMap<_, _>>();

    let mut owned = HashSet::new();
    let (typing_sender, mut typing_receiver) = mpsc::channel(groups.len());
    let mut force_typing = VecDeque::new();

    loop {
        let typing = async {
            if let Some(gid) = force_typing.pop_front() {
                return gid;
            }

            typing_receiver.recv().await.unwrap()
        };

        let event = tokio::select! {
            event = telegram_receiver.recv() => match event {
                Some(event) => Event::Telegram(event),
                None => break,
            },
            update = client.read_update() => Event::Multichat(update?),
            gid = typing => Event::Typing(gid),
        };

        match event {
            Event::Telegram(event) => match event.kind {
                EventKind::Message {
                    user_name,
                    text,
                    attachment,
                } => {
                    let gids = match chat_to_group.get(&event.chat_id) {
                        Some(gids) => gids,
                        None => {
                            tracing::warn!(chat_id = %event.chat_id, "Telegram chat not found");
                            continue;
                        }
                    };

                    let entry = users.entry((event.user_id, event.chat_id));
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
                        client.send_message(*gid, *uid, &text, attachments).await?;
                    }
                }
                EventKind::Leave => {
                    let user = match users.remove(&(event.user_id, event.chat_id)) {
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
                let group = groups.get_mut(&update.gid).unwrap();
                let chat_ids = group_to_chat.get(&update.gid).unwrap();

                match update.kind {
                    UpdateKind::InitUser { uid, name } => {
                        let owned = owned.remove(&(update.gid, uid));
                        let user = group.users.entry(uid).or_insert(MultichatUser {
                            name,
                            owned,
                            typing: false,
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

                        if group.typing.is_some() {
                            force_typing.push_back(update.gid);
                        }
                    }
                    UpdateKind::DestroyUser { uid } => {
                        let user = group.users.remove(&uid).unwrap();
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

                        if !group.users.values().any(|user| user.typing) {
                            let typing = group.typing.take().unwrap();
                            typing.abort();
                            let _ = typing.await;

                            continue;
                        }

                        if group.typing.is_some() {
                            force_typing.push_back(update.gid);
                        }
                    }
                    UpdateKind::Message { uid, message } => {
                        let user = group.users.get(&uid).unwrap();
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

                        if group.typing.is_some() {
                            force_typing.push_back(update.gid);
                        }
                    }
                    UpdateKind::Rename {
                        uid,
                        name: new_name,
                    } => {
                        let user = group.users.get_mut(&uid).unwrap();
                        let old_name = mem::replace(&mut user.name, new_name);

                        if user.owned {
                            continue;
                        }

                        let message = format!(
                            "*{}*: renamed to *{}*",
                            old_name.markdown_safe(),
                            user.name.markdown_safe()
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

                        if group.typing.is_some() {
                            force_typing.push_back(update.gid);
                        }
                    }
                    UpdateKind::StartTyping { uid } => {
                        group.users.get_mut(&uid).unwrap().typing = true;

                        if group.typing.is_some() {
                            continue;
                        }

                        let gid = update.gid;
                        let sender = typing_sender.clone();

                        // Telegram removes the typing indicator after ~5 seconds.
                        group.typing = Some(tokio::spawn(async move {
                            let mut interval = time::interval(Duration::from_secs(5));

                            loop {
                                tokio::select! {
                                    _ = interval.tick() => {
                                        if sender.send(gid).await.is_err() {
                                            break;
                                        }
                                    }
                                    _ = sender.closed() => break,
                                }
                            }
                        }));
                    }
                    UpdateKind::StopTyping { uid } => {
                        let user = group.users.get_mut(&uid).unwrap();
                        user.typing = false;

                        if group.users.values().any(|user| user.typing) {
                            continue;
                        }

                        let typing = group.typing.take().unwrap();
                        typing.abort();
                        let _ = typing.await;
                    }
                    UpdateKind::InitGroup { .. } | UpdateKind::DestroyGroup { .. } => {
                        // Handled above.
                        unreachable!()
                    }
                }
            }
            Event::Typing(gid) => {
                let group = groups.get_mut(&gid).unwrap();
                if group.typing.is_none() {
                    // Harmless race.
                    continue;
                }

                let chat_ids = group_to_chat.get(&gid).unwrap();
                for chat_id in chat_ids {
                    rate_limit(|| async {
                        bot.send_chat_action(*chat_id, ChatAction::Typing).await
                    })
                    .await?;
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
    Typing(u32),
}

struct TelegramUser {
    name: String,
    gid_uid: Vec<(u32, u32)>,
}

struct Group {
    users: HashMap<u32, MultichatUser>,
    typing: Option<JoinHandle<()>>,
}

struct MultichatUser {
    name: String,
    owned: bool,
    typing: bool,
}
