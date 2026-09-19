use crate::{logging, util, Ctx, Error};
use poise::serenity_prelude as serenity;

const BULK_MAX_AGE: i64 = 14 * 86_400 - 600;

/// Delete recent messages
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_MESSAGES")]
pub async fn purge(
    ctx: Ctx<'_>,
    #[description = "How many recent messages to examine (1-500)"]
    #[min = 1]
    #[max = 500]
    scan: u16,
    #[description = "Only this user's messages"] user: Option<serenity::User>,
    #[description = "Only messages matching this regex"] pattern: Option<String>,
    #[description = "Only messages with links, embeds or attachments"] links: Option<bool>,
    #[description = "Only messages from bots"] bots: Option<bool>,
) -> Result<(), Error> {
    let re = match pattern.as_deref() {
        None => None,
        Some(p) => Some(
            regex::RegexBuilder::new(p)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| format!("bad regex: {e}"))?,
        ),
    };

    ctx.defer_ephemeral().await?;
    let channel = ctx.channel_id();
    let cutoff = crate::db::now() - BULK_MAX_AGE;

    let mut targets: Vec<serenity::MessageId> = Vec::new();
    let mut before: Option<serenity::MessageId> = None;
    let mut seen = 0u16;
    let mut too_old = 0u32;

    while seen < scan {
        let batch_size = std::cmp::min(100, scan - seen) as u8;
        let mut req = serenity::GetMessages::new().limit(batch_size);
        if let Some(b) = before {
            req = req.before(b);
        }
        let batch = channel.messages(ctx.http(), req).await?;
        if batch.is_empty() {
            break;
        }
        seen += batch.len() as u16;
        before = batch.last().map(|m| m.id);

        for m in &batch {
            if m.timestamp.unix_timestamp() < cutoff {
                too_old += 1;
                continue;
            }
            if m.pinned {
                continue;
            }
            if let Some(u) = &user {
                if m.author.id != u.id {
                    continue;
                }
            }
            if bots == Some(true) && !m.author.bot {
                continue;
            }
            if links == Some(true)
                && m.attachments.is_empty()
                && m.embeds.is_empty()
                && !m.content.contains("http")
            {
                continue;
            }
            if let Some(re) = &re {
                if !re.is_match(&m.content) {
                    continue;
                }
            }
            targets.push(m.id);
        }
    }

    if targets.is_empty() {
        ctx.say(format!("Nothing matched ({seen} examined, {too_old} too old to bulk delete)"))
            .await?;
        return Ok(());
    }

    let mut deleted = 0usize;
    for chunk in targets.chunks(100) {
        if chunk.len() == 1 {
            channel.delete_message(ctx.http(), chunk[0]).await?;
        } else {
            channel.delete_messages(ctx.http(), chunk).await?;
        }
        deleted += chunk.len();
    }

    let mut note = format!("Deleted {deleted} of {seen} examined");
    if too_old > 0 {
        note.push_str(&format!(" ({too_old} skipped, over 14 days old)"));
    }
    ctx.say(&note).await?;

    let filters = [
        user.as_ref().map(|u| format!("user: {}", u.tag())),
        pattern.map(|p| format!("regex: `{}`", util::clamp_field(&p))),
        links.and_then(|v| v.then(|| "links only".to_string())),
        bots.and_then(|v| v.then(|| "bots only".to_string())),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ");

    let e = serenity::CreateEmbed::new()
        .color(logging::AMBER)
        .title("Purge")
        .field("Channel", format!("<#{channel}>"), true)
        .field("Moderator", ctx.author().tag(), true)
        .field("Deleted", deleted.to_string(), true)
        .field("Filters", if filters.is_empty() { "none".into() } else { filters }, false)
        .timestamp(serenity::Timestamp::now());
    logging::send(ctx.serenity_context(), ctx.data(), e).await;
    Ok(())
}
