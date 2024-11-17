use teloxide::net::Download;
use teloxide::prelude::Requester;
use teloxide::types::{ChatId, MediaKind, MediaText, Message, MessageCommon, MessageKind, UserId};
use teloxide::{Bot, RequestError};
use tokio::sync::mpsc::Sender;

pub struct Event {
    pub chat_id: ChatId,
    pub user_id: UserId,
    pub kind: EventKind,
}

pub enum EventKind {
    Message {
        user_name: String,
        text: String,
        attachment: Option<Vec<u8>>,
    },
    Leave,
}

pub async fn run(bot: Bot, sender: Sender<Event>) {
    teloxide::repl(bot, move |bot: Bot, message: Message| {
        let sender = sender.clone();

        handle(bot, message, sender)
    })
    .await;
}

async fn handle(bot: Bot, message: Message, sender: Sender<Event>) -> Result<(), RequestError> {
    let from = match message.from {
        Some(from) => from,
        None => return Ok(()),
    };

    let chat_id = message.chat.id;
    let (user_id, kind) = match message.kind {
        MessageKind::LeftChatMember(member) => (member.left_chat_member.id, EventKind::Leave),
        MessageKind::Common(MessageCommon { media_kind, .. }) => match media_kind {
            MediaKind::Text(MediaText { text, .. }) => (
                from.id,
                EventKind::Message {
                    user_name: from.full_name(),
                    text,
                    attachment: None,
                },
            ),
            MediaKind::Photo(photo) => {
                let text = photo.caption.unwrap_or_default();
                let photo = photo
                    .photo
                    .into_iter()
                    .max_by_key(|photo| photo.width * photo.height);

                let attachment = match photo {
                    Some(photo) => {
                        let mut data = Vec::new();

                        let file = bot.get_file(&photo.file.id).await?;
                        bot.download_file(&file.path, &mut data).await?;

                        Some(data)
                    }
                    None => None,
                };

                (
                    from.id,
                    EventKind::Message {
                        user_name: from.full_name(),
                        text,
                        attachment,
                    },
                )
            }
            MediaKind::Video(video) => {
                let mut data = Vec::new();

                let file = bot.get_file(&video.video.file.id).await?;
                bot.download_file(&file.path, &mut data).await?;

                (
                    from.id,
                    EventKind::Message {
                        user_name: from.full_name(),
                        text: video.caption.unwrap_or_default(),
                        attachment: Some(data),
                    },
                )
            }
            MediaKind::Document(document) => {
                let mut data = Vec::new();

                let file = bot.get_file(&document.document.file.id).await?;
                bot.download_file(&file.path, &mut data).await?;

                (
                    from.id,
                    EventKind::Message {
                        user_name: from.full_name(),
                        text: document.caption.unwrap_or_default(),
                        attachment: Some(data),
                    },
                )
            }
            MediaKind::Voice(voice) => {
                let mut data = Vec::new();

                let file = bot.get_file(&voice.voice.file.id).await?;
                bot.download_file(&file.path, &mut data).await?;

                (
                    from.id,
                    EventKind::Message {
                        user_name: from.full_name(),
                        text: voice.caption.unwrap_or_default(),
                        attachment: Some(data),
                    },
                )
            }
            _ => return Ok(()),
        },
        _ => return Ok(()),
    };

    let event = Event {
        chat_id,
        user_id,
        kind,
    };

    let _ = sender.send(event).await;

    Ok(())
}
