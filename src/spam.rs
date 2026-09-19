use crate::{db, logging, util, Data, Error};
use poise::serenity_prelude as serenity;
use std::sync::{Arc, Mutex, OnceLock};

pub type Db = Arc<Mutex<rusqlite::Connection>>;
pub type Slot = Arc<Mutex<Option<serenity::ChannelId>>>;
pub type Windows = Arc<Mutex<std::collections::HashMap<u64, Vec<i64>>>>;

pub const IMAGE_WINDOW_MS: i64 = 5_000;

const EXEMPT_STAFF: bool = true;
const IMAGE_LIMIT: usize = 4;
const CONTACT: &str = "germ99__";
const MIRROR_MAX: usize = 4000;

const K_CHANNEL: &str = "honeypot_channel";
const K_MESSAGE: &str = "honeypot_message";
const K_DAY: &str = "honeypot_day";

const STAFF: serenity::Permissions = serenity::Permissions::ADMINISTRATOR
    .union(serenity::Permissions::BAN_MEMBERS)
    .union(serenity::Permissions::KICK_MEMBERS)
    .union(serenity::Permissions::MODERATE_MEMBERS)
    .union(serenity::Permissions::MANAGE_MESSAGES);

fn today() -> i64 {
    db::now() / 86_400
}

fn daily_name(guild: serenity::GuildId, day: i64) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut x = (day as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(guild.get())
        | 1;
    (0..10)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            ALPHABET[(x % ALPHABET.len() as u64) as usize] as char
        })
        .collect()
}

fn warning_embed() -> serenity::CreateEmbed {
    serenity::CreateEmbed::new()
        .color(0xE0_52_52)
        .title("DO NOT POST IN THIS CHANNEL")
        .description(
            "**Anyone who sends a message here is permanently banned, automatically \
             and immediately.**\n\n\
             This channel exists only to catch automated accounts. A real member has no reason to type here.\n\n\
             Do not post. Do not test it. Do not react to a dare.\n\n\
             *Nothing you send here will be read by a human.*",
        )
        .footer(serenity::CreateEmbedFooter::new(
            "This channel is renamed every day. It is always this channel.",
        ))
}

// never thought about it before, but man do I hate regex
fn invite_re() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| {
        regex::RegexBuilder::new(
            r"(?:discord(?:app)?\.com/invite|discord\.gg|discord\.me|dsc\.gg|invite\.gg)/[a-z0-9\-_]+",
        )
        .case_insensitive(true)
        .build()
        .expect("invite pattern")
    })
}

fn is_staff(ctx: &serenity::Context, guild: serenity::GuildId, user: serenity::UserId) -> bool {
    let Some(g) = ctx.cache.guild(guild) else {
        return false;
    };
    if g.owner_id == user {
        return true;
    }
    let Some(member) = g.members.get(&user) else {
        return false;
    };
    member
        .roles
        .iter()
        .filter_map(|id| g.roles.get(id))
        .any(|r| r.permissions.intersects(STAFF))
}

pub async fn ensure_channel(
    ctx: &serenity::Context,
    db: &Db,
    guild: serenity::GuildId,
    slot: &Slot,
) -> Result<serenity::ChannelId, Error> {
    let day = today();
    let name = daily_name(guild, day);

    let stored = {
        let c = db.lock().unwrap();
        (
            db::kv_u64(&c, K_CHANNEL),
            db::kv_u64(&c, K_MESSAGE),
            db::kv_u64(&c, K_DAY),
        )
    };

    let channel = match stored.0.map(serenity::ChannelId::new) {
        Some(id) if id.to_channel(&ctx.http).await.is_ok() => id,
        _ => {
            let made = guild
                .create_channel(
                    &ctx.http,
                    serenity::CreateChannel::new(&name)
                        .kind(serenity::ChannelType::Text)
                        .topic("Do not post here."),
                )
                .await?;
            let c = db.lock().unwrap();
            db::kv_set(&c, K_CHANNEL, &made.id.get().to_string())?;
            db::kv_set(&c, K_DAY, &day.to_string())?;
            made.id
        }
    };

    if stored.2 != Some(day as u64) {
        channel
            .edit(&ctx.http, serenity::EditChannel::new().name(&name))
            .await?;
        let c = db.lock().unwrap();
        db::kv_set(&c, K_DAY, &day.to_string())?;
    }

    let has_warning = match stored.1.map(serenity::MessageId::new) {
        Some(id) => channel.message(&ctx.http, id).await.is_ok(),
        None => false,
    };
    if !has_warning {
        let msg = channel
            .send_message(
                &ctx.http,
                serenity::CreateMessage::new().embed(warning_embed()),
            )
            .await?;
        let _ = msg.pin(&ctx.http).await;
        let c = db.lock().unwrap();
        db::kv_set(&c, K_MESSAGE, &msg.id.get().to_string())?;
    }

    *slot.lock().unwrap() = Some(channel);
    Ok(channel)
}

pub fn spawn_rotation(ctx: serenity::Context, db: Db, guild: serenity::GuildId, slot: Slot) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1800));
        loop {
            tick.tick().await;
            if let Err(e) = ensure_channel(&ctx, &db, guild, &slot).await {
                eprintln!("honeypot rotation failed: {e}");
            }
        }
    });
}

pub async fn on_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    let Some(guild) = msg.guild_id else {
        return Ok(());
    };
    if msg.author.id == ctx.cache.current_user().id {
        return Ok(());
    }

    let honeypot = *data.honeypot.lock().unwrap();
    if honeypot == Some(msg.channel_id) {
        return honeypot_trip(ctx, msg, data, guild).await;
    }

    if msg.author.bot {
        return Ok(());
    }

    mirror(msg, data).await?;
    if EXEMPT_STAFF && is_staff(ctx, guild, msg.author.id) {
        return Ok(());
    }

    if invite_re().is_match(&msg.content) {
        return invite_hit(ctx, msg, data).await;
    }
    image_flood(ctx, msg, data).await
}

async fn honeypot_trip(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
    guild: serenity::GuildId,
) -> Result<(), Error> {
    if msg.author.bot {
        let _ = msg.delete(&ctx.http).await;
        return Ok(());
    }

    if EXEMPT_STAFF && is_staff(ctx, guild, msg.author.id) {
        let _ = msg.delete(&ctx.http).await;
        let e = serenity::CreateEmbed::new()
            .color(logging::AMBER)
            .title("Honeypot tripped by staff")
            .field(
                "User",
                format!("{} `{}`", msg.author.tag(), msg.author.id),
                true,
            )
            .field("Action", "none, exempt", true)
            .field("Content", util::clamp_field(&msg.content), false)
            .timestamp(serenity::Timestamp::now());
        logging::send(ctx, data, e).await;
        return Ok(());
    }

    let reason = "Posted in the honeypot channel";

    let _ = msg
        .author
        .direct_message(
            ctx,
            serenity::CreateMessage::new().embed(
                serenity::CreateEmbed::new()
                    .color(logging::RED)
                    .title("You have been permanently banned")
                    .description(format!(
                        "You sent a message in a channel that was a honeypot \n
                         **If you believe this was a mistake, message `{CONTACT}` on Discord.**"
                    )),
            ),
        )
        .await;

    guild
        .ban_with_reason(&ctx.http, msg.author.id, 1, reason)
        .await?;

    let (target, actor) = (msg.author.id.get(), ctx.cache.current_user().id.get());
    let case = data
        .with_db(move |c| db::add(c, target, actor, "ban", "Posted in the honeypot channel", None))
        .await?;

    let e = serenity::CreateEmbed::new()
        .color(logging::RED)
        .title(format!("Honeypot ban - case #{case}"))
        .field(
            "User",
            format!("{} `{}`", msg.author.tag(), msg.author.id),
            true,
        )
        .field("Account age", util::fmt_duration(db::now() - msg.author.id.created_at().unix_timestamp()), true)
        .field("Content", util::clamp_field(&msg.content), false)
        .timestamp(serenity::Timestamp::now());
    logging::send(ctx, data, e).await;
    Ok(())
}

async fn invite_hit(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    let _ = msg.delete(&ctx.http).await;
    let reason = "Posted a Discord invite link";

    let (target, actor) = (msg.author.id.get(), ctx.cache.current_user().id.get());
    let case = data
        .with_db(move |c| db::add(c, target, actor, "invite", reason, None))
        .await?;

    let _ = msg
        .author
        .direct_message(
            ctx,
            serenity::CreateMessage::new().content(format!(
                "Your message was removed: invite links aren't allowed here. \
                 This has been recorded as case #{case}."
            )),
        )
        .await;

    let e = serenity::CreateEmbed::new()
        .color(logging::AMBER)
        .title(format!("Invite removed - case #{case}"))
        .field(
            "User",
            format!("{} `{}`", msg.author.tag(), msg.author.id),
            true,
        )
        .field("Channel", format!("<#{}>", msg.channel_id), true)
        .field("Content", util::clamp_field(&msg.content), false)
        .timestamp(serenity::Timestamp::now());
    logging::send(ctx, data, e).await;
    Ok(())
}

async fn image_flood(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    let images = msg
        .attachments
        .iter()
        .filter(|a| {
            a.content_type
                .as_deref()
                .is_some_and(|t| t.starts_with("image/"))
        })
        .count();
    if images == 0 {
        return Ok(());
    }

    let now = db::now_ms();
    let tripped = {
        let mut seen = data.images.lock().unwrap();
        let stamps = seen.entry(msg.author.id.get()).or_default();
        stamps.retain(|t| now - *t < IMAGE_WINDOW_MS);
        stamps.extend(std::iter::repeat(now).take(images));
        if stamps.len() >= IMAGE_LIMIT {
            stamps.clear();
            true
        } else {
            false
        }
    };
    if !tripped {
        return Ok(());
    }

    let _ = msg.delete(&ctx.http).await;
    let reason = format!("{IMAGE_LIMIT}+ images within {}s", IMAGE_WINDOW_MS / 1000);

    let (target, actor) = (msg.author.id.get(), ctx.cache.current_user().id.get());
    let case = data
        .with_db(move |c| db::add(c, target, actor, "imagespam", &reason, None))
        .await?;

    let e = serenity::CreateEmbed::new()
        .color(logging::AMBER)
        .title(format!("Image flood - case #{case}"))
        .field(
            "User",
            format!("{} `{}`", msg.author.tag(), msg.author.id),
            true,
        )
        .field("Channel", format!("<#{}>", msg.channel_id), true)
        .field("Images in message", images.to_string(), true)
        .timestamp(serenity::Timestamp::now());
    logging::send(ctx, data, e).await;
    Ok(())
}

async fn mirror(msg: &serenity::Message, data: &Data) -> Result<(), Error> {
    let files = msg
        .attachments
        .iter()
        .map(|a| a.filename.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let content: String = msg.content.chars().take(MIRROR_MAX).collect();
    let (id, channel, author) = (
        msg.id.get(),
        msg.channel_id.get(),
        msg.author.id.get(),
    );
    data.with_db(move |c| db::cache_message(c, id, channel, author, &content, &files))
        .await?;
    Ok(())
}
