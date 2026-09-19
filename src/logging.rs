use crate::{db, util, Data, Error};
use poise::serenity_prelude as serenity;

pub const RED: u32 = 0xE05252;
pub const AMBER: u32 = 0xE0A352;
pub const GREEN: u32 = 0x52E086;
pub const GREY: u32 = 0x8A8A8A;

pub async fn send(ctx: &serenity::Context, data: &Data, embed: serenity::CreateEmbed) {
    if let Err(e) = data
        .log_channel
        .send_message(&ctx.http, serenity::CreateMessage::new().embed(embed))
        .await
    {
        eprintln!("log channel write failed: {e}");
    }
}

/// Record of a moderator action, written to the log channel
pub async fn action(
    ctx: &serenity::Context,
    data: &Data,
    color: u32,
    title: &str,
    target: &serenity::User,
    moderator: &serenity::User,
    reason: &str,
    extra: Option<(&str, String)>,
    case: Option<i64>,
) {
    let mut e = serenity::CreateEmbed::new()
        .color(color)
        .title(match case {
            Some(id) => format!("{title} - case #{id}"),
            None => title.to_string(),
        })
        .field("User", format!("{} `{}`", target.tag(), target.id), true)
        .field("Moderator", moderator.tag(), true)
        .field("Reason", util::clamp_field(reason), false)
        .timestamp(serenity::Timestamp::now());
    if let Some((name, value)) = extra {
        e = e.field(name, value, true);
    }
    send(ctx, data, e).await;
}

pub async fn handle(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    data: &Data,
) -> Result<(), Error> {
    use serenity::FullEvent as E;
    match event {
        E::Message { new_message } => {
            crate::spam::on_message(ctx, new_message, data).await?;
        }

        E::MessageDelete {
            channel_id,
            deleted_message_id,
            guild_id,
        } => {
            let honeypot = *data.honeypot.lock().unwrap();
            if guild_id.is_none()
                || *channel_id == data.log_channel
                || honeypot == Some(*channel_id)
            {
                return Ok(());
            }
            let cached = ctx
                .cache
                .message(*channel_id, *deleted_message_id)
                .map(|m| (m.author.id.get(), m.content.clone(), m.attachments.iter().map(|a| a.filename.clone()).collect::<Vec<_>>().join(", "), m.author.bot));

            let (author_id, content, files) = match cached {
                Some((_, _, _, true)) => return Ok(()),
                Some((id, content, files, false)) => (id, content, files),
                None => {
                    let id = deleted_message_id.get();
                    let Some(row) = data.with_db(move |c| db::cached_message(c, id)).await? else {
                        return Ok(());
                    };
                    (row.author_id, row.content, row.files)
                }
            };

            let body = if content.is_empty() {
                "*(no text)*".to_string()
            } else {
                util::clamp_field(&content)
            };
            let mut e = serenity::CreateEmbed::new()
                .color(RED)
                .title("Message deleted")
                .field("Author", format!("<@{author_id}> `{author_id}`"), true)
                .field("Channel", format!("<#{channel_id}>"), true)
                .field("Content", body, false)
                .timestamp(serenity::Timestamp::now());
            if !files.is_empty() {
                e = e.field("Attachments", util::clamp_field(&files), false);
            }
            send(ctx, data, e).await;
        }

        E::MessageUpdate {
            old_if_available,
            new,
            event,
        } => {
            if event.guild_id.is_none() || event.channel_id == data.log_channel {
                return Ok(());
            }
            let Some(new) = new else { return Ok(()) };
            if new.author.bot {
                return Ok(());
            }
            let before = match old_if_available.as_ref().map(|m| m.content.clone()) {
                Some(c) => c,
                None => {
                    let id = new.id.get();
                    data.with_db(move |c| db::cached_message(c, id))
                        .await?
                        .map(|r| r.content)
                        .unwrap_or_else(|| "*(not cached)*".into())
                }
            };
            if before == new.content {
                return Ok(());
            }
            let e = serenity::CreateEmbed::new()
                .color(AMBER)
                .title("Message edited")
                .url(new.link())
                .field("Author", format!("{} `{}`", new.author.tag(), new.author.id), true)
                .field("Channel", format!("<#{}>", new.channel_id), true)
                .field("Before", util::clamp_field(&before), false)
                .field("After", util::clamp_field(&new.content), false)
                .timestamp(serenity::Timestamp::now());
            send(ctx, data, e).await;

            let (id, channel, author, content) = (
                new.id.get(),
                new.channel_id.get(),
                new.author.id.get(),
                new.content.clone(),
            );
            data.with_db(move |c| db::cache_message(c, id, channel, author, &content, ""))
                .await?;
        }

        E::GuildMemberAddition { new_member } => {
            let age = db_age(new_member.user.id);
            let uid = new_member.user.id.get();
            let records = data.with_db(move |c| db::count(c, uid)).await.unwrap_or(0);
            let mut e = serenity::CreateEmbed::new()
                .color(GREEN)
                .title("Member joined")
                .field(
                    "User",
                    format!("{} `{}`", new_member.user.tag(), new_member.user.id),
                    true,
                )
                .field("Account age", age, true)
                .timestamp(serenity::Timestamp::now());
            if records > 0 {
                e = e.field("Prior records", records.to_string(), true);
            }
            send(ctx, data, e).await;
        }

        E::GuildMemberRemoval { user, .. } => {
            let e = serenity::CreateEmbed::new()
                .color(GREY)
                .title("Member left")
                .field("User", format!("{} `{}`", user.tag(), user.id), true)
                .timestamp(serenity::Timestamp::now());
            send(ctx, data, e).await;
        }

        _ => {}
    }
    Ok(())
}

fn db_age(id: serenity::UserId) -> String {
    let created = id.created_at().unix_timestamp();
    util::fmt_duration(crate::db::now() - created)
}
