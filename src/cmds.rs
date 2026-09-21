use crate::{db, logging, util, Ctx, Error};
use poise::serenity_prelude as serenity;

const MAX_TIMEOUT: i64 = 28 * 86_400;

const NO_REASON: &str = "*(no reason given)*";

fn reason_or_blank(r: Option<String>) -> String {
    r.map(|s| util::clamp_reason(&s)).unwrap_or_default()
}

/// Log lines that aren't about a specific punished user
pub async fn log_note(ctx: &Ctx<'_>, color: u32, title: &str, fields: Vec<(&str, String, bool)>) {
    let mut e = serenity::CreateEmbed::new()
        .color(color)
        .title(title)
        .field("Moderator", ctx.author().tag(), true)
        .timestamp(serenity::Timestamp::now());
    for (name, value, inline) in fields {
        e = e.field(name, value, inline);
    }
    logging::send(ctx.serenity_context(), ctx.data(), e).await;
}

fn show(reason: &str) -> &str {
    if reason.is_empty() {
        NO_REASON
    } else {
        reason
    }
}

pub fn all() -> Vec<poise::Command<crate::Data, Error>> {
    vec![
        warn(), note(), records(), cases(), case(), clearrecords(),
        mute(), unmute(), kick(), ban(), softban(), unban(),
        slowmode(), lock(), unlock(), whois(),
        crate::purge::purge(),
        crate::tickets::ticket(),
    ]
}

/// a closed DM must not stop the punishment landing
async fn notify(ctx: &Ctx<'_>, user: &serenity::User, body: String) -> bool {
    user.direct_message(
        ctx.serenity_context(),
        serenity::CreateMessage::new().content(body),
    )
    .await
    .is_ok()
}

fn guild(ctx: &Ctx<'_>) -> Result<serenity::GuildId, Error> {
    ctx.guild_id().ok_or_else(|| "guild only".into())
}

fn top_role(guild: &serenity::Guild, m: &serenity::Member) -> i64 {
    m.roles
        .iter()
        .filter_map(|id| guild.roles.get(id))
        .map(|r| r.position as i64)
        .max()
        .unwrap_or(-1)
}

/// Blocks a mod from acting on someone at or above their own role height, and on the bot itself
async fn hierarchy_ok(ctx: &Ctx<'_>, target: serenity::UserId) -> Result<(), Error> {
    if target == ctx.framework().bot_id {
        return Err("that's me".into());
    }
    if target == ctx.author().id {
        return Err("that's you".into());
    }
    let g = guild(ctx)?;
    let Ok(target) = g.member(ctx, target).await else {
        return Ok(());
    };
    let author = g.member(ctx, ctx.author().id).await?;

    let Some(gd) = ctx.guild() else { return Ok(()) };
    if gd.owner_id == author.user.id || top_role(&gd, &author) > top_role(&gd, &target) {
        Ok(())
    } else {
        Err("that user is at or above you in the role list".into())
    }
}

/// Warn a user. Records it and sends them a DM
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn warn(
    ctx: Ctx<'_>,
    #[description = "User to warn"] user: serenity::User,
    #[description = "Why (can be filled in later with /case reason)"] reason: Option<String>,
) -> Result<(), Error> {
    hierarchy_ok(&ctx, user.id).await?;
    let reason = reason_or_blank(reason);

    let (uid, mid, r) = (user.id.get(), ctx.author().id.get(), reason.clone());
    let (case, total) = ctx
        .data()
        .with_db(move |c| {
            let id = db::add(c, uid, mid, "warn", &r, None)?;
            Ok((id, db::count(c, uid)?))
        })
        .await?;

    let guild_name = ctx.guild().map(|g| g.name.clone()).unwrap_or_default();
    let dm = notify(&ctx, &user, format!("You were warned in **{guild_name}**: {}", show(&reason))).await;

    ctx.say(format!(
        "Warned {} — case #{case} ({total} record{}){}",
        user.tag(),
        if total == 1 { "" } else { "s" },
        if dm { "" } else { " — DM failed" }
    ))
    .await?;
    logging::action(ctx.serenity_context(), ctx.data(), logging::AMBER, "Warn", &user, ctx.author(), show(&reason), None, Some(case)).await;
    Ok(())
}

/// Attach a private note to a user. No DM is sent
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn note(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
    #[description = "Note text"] text: String,
) -> Result<(), Error> {
    let text = util::clamp_reason(&text);
    let (uid, mid, t) = (user.id.get(), ctx.author().id.get(), text.clone());
    let case = ctx
        .data()
        .with_db(move |c| db::add(c, uid, mid, "note", &t, None))
        .await?;
    ctx.say(format!("Noted on {} — case #{case}", user.tag())).await?;
    logging::action(ctx.serenity_context(), ctx.data(), logging::GREY, "Note", &user, ctx.author(), &text, None, Some(case)).await;
    Ok(())
}

/// Show a user's record history, newest first
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn records(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
) -> Result<(), Error> {
    let uid = user.id.get();
    let rows = ctx.data().with_db(move |c| db::list(c, uid, 25)).await?;
    if rows.is_empty() {
        ctx.say(format!("{} has no records", user.tag())).await?;
        return Ok(());
    }

    let body: String = rows
        .iter()
        .map(|r| {
            let when = format!("<t:{}:R>", r.created);
            let dur = match r.expires {
                Some(e) if r.undone => format!(" [{} - lifted]", util::fmt_duration(e - r.created)),
                Some(e) => format!(" [{}]", util::fmt_duration(e - r.created)),
                None => String::new(),
            };
            format!("`#{}` **{}**{} by <@{}> {}\n{}", r.id, r.kind, dur, r.mod_id, when, show(&r.reason))
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let e = serenity::CreateEmbed::new()
        .title(format!("Records for {}", user.tag()))
        .description(util::clamp_desc(&body))
        .footer(serenity::CreateEmbedFooter::new(format!("id {}", user.id)));
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Work with an individual case
#[poise::command(
    slash_command,
    guild_only,
    subcommands("case_view", "case_reason", "case_delete"),
    subcommand_required
)]
pub async fn case(_: Ctx<'_>) -> Result<(), Error> {
    Ok(())
}

/// View one case by number
#[poise::command(slash_command, rename = "view", required_permissions = "MODERATE_MEMBERS")]
pub async fn case_view(
    ctx: Ctx<'_>,
    #[description = "Case number"] id: i64,
) -> Result<(), Error> {
    let r = ctx.data().with_db(move |c| db::get(c, id)).await?;
    let Some(r) = r else {
        ctx.say(format!("No case #{id}")).await?;
        return Ok(());
    };

    let mut e = serenity::CreateEmbed::new()
        .title(format!("Case #{} - {}", r.id, r.kind))
        .field("User", format!("<@{}> `{}`", r.user_id, r.user_id), true)
        .field("Moderator", format!("<@{}>", r.mod_id), true)
        .field("When", format!("<t:{}:F>", r.created), false)
        .field("Reason", util::clamp_field(show(&r.reason)), false);
    if let Some(exp) = r.expires {
        e = e.field(
            "Duration",
            format!(
                "{}{}",
                util::fmt_duration(exp - r.created),
                if r.undone { " (lifted)" } else { "" }
            ),
            true,
        );
    }
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

/// Set or replace a case's reason, for when you had to act before you could explain
#[poise::command(slash_command, rename = "reason", required_permissions = "MODERATE_MEMBERS")]
pub async fn case_reason(
    ctx: Ctx<'_>,
    #[description = "Case number"] id: i64,
    #[description = "Reason"] reason: String,
) -> Result<(), Error> {
    let reason = util::clamp_reason(&reason);
    let existing = ctx.data().with_db(move |c| db::get(c, id)).await?;
    let Some(existing) = existing else {
        ctx.say(format!("No case #{id}")).await?;
        return Ok(());
    };
    let r = reason.clone();
    ctx.data().with_db(move |c| db::set_reason(c, id, &r)).await?;

    ctx.say(format!(
        "Case #{id} ({} on <@{}>) now reads: {reason}",
        existing.kind, existing.user_id
    ))
    .await?;

    let e = serenity::CreateEmbed::new()
        .color(logging::GREY)
        .title(format!("Reason set - case #{id}"))
        .field("User", format!("<@{}>", existing.user_id), true)
        .field("Moderator", ctx.author().tag(), true)
        .field("Was", util::clamp_field(show(&existing.reason)), false)
        .field("Now", util::clamp_field(&reason), false)
        .timestamp(serenity::Timestamp::now());
    logging::send(ctx.serenity_context(), ctx.data(), e).await;
    Ok(())
}

/// Remove a case from the record entirely
#[poise::command(slash_command, rename = "delete", required_permissions = "MODERATE_MEMBERS")]
pub async fn case_delete(
    ctx: Ctx<'_>,
    #[description = "Case number"] id: i64,
) -> Result<(), Error> {
    let existing = ctx
        .data()
        .with_db(move |c| {
            let r = db::get(c, id)?;
            if r.is_some() {
                db::delete(c, id)?;
            }
            Ok(r)
        })
        .await?;
    let Some(r) = existing else {
        ctx.say(format!("No case #{id}")).await?;
        return Ok(());
    };
    ctx.say(format!("Deleted case #{id}")).await?;

    log_note(
        &ctx,
        logging::RED,
        &format!("Case deleted - #{id}"),
        vec![
            ("User", format!("<@{}> `{}`", r.user_id, r.user_id), true),
            ("Kind", r.kind.clone(), true),
            ("Originally by", format!("<@{}>", r.mod_id), true),
            ("Reason", util::clamp_field(show(&r.reason)), false),
        ],
    )
    .await;
    Ok(())
}

/// Wipe every record for a user
#[poise::command(slash_command, guild_only, required_permissions = "BAN_MEMBERS")]
pub async fn clearrecords(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
) -> Result<(), Error> {
    let uid = user.id.get();
    let (rows, n) = ctx
        .data()
        .with_db(move |c| {
            let rows = db::list(c, uid, i64::MAX)?;
            Ok((rows, db::clear(c, uid)?))
        })
        .await?;
    ctx.say(format!("Cleared {n} record(s) for {}", user.tag())).await?;

    let mut summary = rows
        .iter()
        .map(|r| format!("`#{}` {} — {}", r.id, r.kind, show(&r.reason)))
        .collect::<Vec<_>>()
        .join("\n");
    if n > rows.len() {
        summary.push_str(&format!("\n… and {} older", n - rows.len()));
    }
    log_note(
        &ctx,
        logging::RED,
        "Records cleared",
        vec![
            ("User", format!("<@{}> `{}`", user.id, user.id), true),
            ("Count", n.to_string(), true),
            ("Destroyed", util::clamp_field(if summary.is_empty() { "nothing" } else { &summary }), false),
        ],
    )
    .await;
    Ok(())
}

/// Time a user out. Duration is required: 30m, 2h, 1d12h, or a bare number of minutes
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn mute(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
    #[description = "30m / 2h / 1d12h"] duration: String,
    #[description = "Why (can be filled in later with /case reason)"] reason: Option<String>,
) -> Result<(), Error> {
    hierarchy_ok(&ctx, user.id).await?;
    let secs = util::parse_duration(&duration)
        .ok_or("bad duration, try 30m / 2h / 1d12h (bare numbers mean minutes)")?;
    if secs > MAX_TIMEOUT {
        return Err("Discord caps timeouts at 28 days".into());
    }
    let reason = reason_or_blank(reason);
    let until = serenity::Timestamp::from_unix_timestamp(db::now() + secs)?;

    let mut member = guild(&ctx)?.member(ctx, user.id).await?;
    member
        .disable_communication_until_datetime(ctx.http(), until)
        .await?;

    let (uid, mid, r) = (user.id.get(), ctx.author().id.get(), reason.clone());
    let until_ts = db::now() + secs;
    let case = ctx
        .data()
        .with_db(move |c| db::add(c, uid, mid, "mute", &r, Some(until_ts)))
        .await?;
    notify(&ctx, &user, format!("You were muted for {}: {}", util::fmt_duration(secs), show(&reason))).await;

    ctx.say(format!("Muted {} for {} — case #{case}", user.tag(), util::fmt_duration(secs))).await?;
    logging::action(
        ctx.serenity_context(), ctx.data(), logging::AMBER, "Mute", &user, ctx.author(), show(&reason),
        Some(("Duration", util::fmt_duration(secs))), Some(case),
    ).await;
    Ok(())
}

/// Lift a timeout early
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn unmute(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
) -> Result<(), Error> {
    let mut member = guild(&ctx)?.member(ctx, user.id).await?;
    member.enable_communication(ctx.http()).await?;
    ctx.say(format!("Unmuted {}", user.tag())).await?;
    logging::action(ctx.serenity_context(), ctx.data(), logging::GREEN, "Unmute", &user, ctx.author(), "", None, None).await;
    Ok(())
}

#[poise::command(slash_command, guild_only, required_permissions = "KICK_MEMBERS")]
pub async fn kick(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
    #[description = "Why (can be filled in later with /case reason)"] reason: Option<String>,
) -> Result<(), Error> {
    hierarchy_ok(&ctx, user.id).await?;
    let reason = reason_or_blank(reason);

    notify(&ctx, &user, format!("You were kicked: {}", show(&reason))).await;
    guild(&ctx)?.kick_with_reason(ctx.http(), user.id, show(&reason)).await?;

    let (uid, mid, r) = (user.id.get(), ctx.author().id.get(), reason.clone());
    let case = ctx
        .data()
        .with_db(move |c| db::add(c, uid, mid, "kick", &r, None))
        .await?;
    ctx.say(format!("Kicked {} — case #{case}", user.tag())).await?;
    logging::action(ctx.serenity_context(), ctx.data(), logging::RED, "Kick", &user, ctx.author(), show(&reason), None, Some(case)).await;
    Ok(())
}

/// Ban a user. Leave duration empty for permanent. Works on users not in the server
#[poise::command(slash_command, guild_only, required_permissions = "BAN_MEMBERS")]
pub async fn ban(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
    #[description = "Why (can be filled in later with /case reason)"] reason: Option<String>,
    #[description = "7d / 12h — omit for permanent"] duration: Option<String>,
    #[description = "Days of their messages to delete (0-7)"]
    #[min = 0]
    #[max = 7]
    purge_days: Option<u8>,
) -> Result<(), Error> {
    hierarchy_ok(&ctx, user.id).await?;
    let reason = reason_or_blank(reason);
    let secs = match duration.as_deref() {
        None => None,
        Some(d) => Some(
            util::parse_duration(d).ok_or("bad duration, try 7d / 12h / 1d12h")?,
        ),
    };

    let human = secs.map(util::fmt_duration).unwrap_or_else(|| "permanent".into());
    notify(&ctx, &user, format!("You were banned ({human}): {}", show(&reason))).await;

    guild(&ctx)?
        .ban_with_reason(ctx.http(), user.id, purge_days.unwrap_or(0), show(&reason))
        .await?;

    let (uid, mid, r) = (user.id.get(), ctx.author().id.get(), reason.clone());
    let expires = secs.map(|s| db::now() + s);
    let case = ctx
        .data()
        .with_db(move |c| db::add(c, uid, mid, "ban", &r, expires))
        .await?;
    if expires.is_some() {
        ctx.data().unban_wake.notify_one();
    }
    ctx.say(format!("Banned {} ({human}) — case #{case}", user.tag())).await?;
    logging::action(
        ctx.serenity_context(), ctx.data(), logging::RED, "Ban", &user, ctx.author(), show(&reason),
        Some(("Duration", human)), Some(case),
    ).await;
    Ok(())
}

/// Ban then immediately unban, to wipe a user's recent messages without keeping them out
#[poise::command(slash_command, guild_only, required_permissions = "BAN_MEMBERS")]
pub async fn softban(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
    #[description = "Why (can be filled in later with /case reason)"] reason: Option<String>,
    #[description = "Days of messages to delete (0-7, default 1)"]
    #[min = 0]
    #[max = 7]
    purge_days: Option<u8>,
) -> Result<(), Error> {
    hierarchy_ok(&ctx, user.id).await?;
    let reason = reason_or_blank(reason);
    let g = guild(&ctx)?;

    notify(&ctx, &user, format!("You were softbanned (messages removed, you may rejoin): {}", show(&reason))).await;
    g.ban_with_reason(ctx.http(), user.id, purge_days.unwrap_or(1), show(&reason)).await?;
    g.unban(ctx.http(), user.id).await?;

    let (uid, mid, r) = (user.id.get(), ctx.author().id.get(), reason.clone());
    let case = ctx
        .data()
        .with_db(move |c| db::add(c, uid, mid, "softban", &r, None))
        .await?;
    ctx.say(format!("Softbanned {} — case #{case}", user.tag())).await?;
    logging::action(ctx.serenity_context(), ctx.data(), logging::RED, "Softban", &user, ctx.author(), show(&reason), None, Some(case)).await;
    Ok(())
}

/// Lift a ban. Takes a raw user id, since the user isn't in the server to pick
#[poise::command(slash_command, guild_only, required_permissions = "BAN_MEMBERS")]
pub async fn unban(
    ctx: Ctx<'_>,
    #[description = "User id"] user_id: String,
) -> Result<(), Error> {
    let id: u64 = user_id.trim().parse().map_err(|_| "that isn't a user id")?;
    let id = serenity::UserId::new(id);
    guild(&ctx)?.unban(ctx.http(), id).await?;

    let uid = id.get();
    ctx.data().with_db(move |c| db::mark_bans_undone(c, uid)).await?;
    ctx.say(format!("Unbanned `{id}`")).await?;
    Ok(())
}

/// Set this channel's slowmode. 0 turns it off
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_CHANNELS")]
pub async fn slowmode(
    ctx: Ctx<'_>,
    #[description = "Seconds between messages, 0 to disable (max 21600)"]
    #[min = 0]
    #[max = 21600]
    seconds: u16,
) -> Result<(), Error> {
    ctx.channel_id()
        .edit(ctx.http(), serenity::EditChannel::new().rate_limit_per_user(seconds))
        .await?;
    ctx.say(if seconds == 0 {
        "Slowmode off".to_string()
    } else {
        format!("Slowmode set to {seconds}s")
    })
    .await?;

    log_note(
        &ctx,
        logging::GREY,
        "Slowmode",
        vec![
            ("Channel", format!("<#{}>", ctx.channel_id()), true),
            ("Interval", if seconds == 0 { "off".into() } else { format!("{seconds}s") }, true),
        ],
    )
    .await;
    Ok(())
}

async fn set_lock(ctx: &Ctx<'_>, locked: bool) -> Result<(), Error> {
    let g = guild(ctx)?;
    let perms = serenity::Permissions::SEND_MESSAGES
        | serenity::Permissions::SEND_MESSAGES_IN_THREADS
        | serenity::Permissions::ADD_REACTIONS;
    let (allow, deny) = if locked {
        (serenity::Permissions::empty(), perms)
    } else {
        (serenity::Permissions::empty(), serenity::Permissions::empty())
    };
    ctx.channel_id()
        .create_permission(
            ctx.http(),
            serenity::PermissionOverwrite {
                allow,
                deny,
                kind: serenity::PermissionOverwriteType::Role(serenity::RoleId::new(g.get())),
            },
        )
        .await?;
    Ok(())
}

/// Stop @everyone posting in this channel
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_CHANNELS")]
pub async fn lock(ctx: Ctx<'_>) -> Result<(), Error> {
    set_lock(&ctx, true).await?;
    ctx.say("Channel locked").await?;
    log_note(&ctx, logging::AMBER, "Channel locked",
        vec![("Channel", format!("<#{}>", ctx.channel_id()), true)]).await;
    Ok(())
}

/// Clear the lock
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_CHANNELS")]
pub async fn unlock(ctx: Ctx<'_>) -> Result<(), Error> {
    set_lock(&ctx, false).await?;
    ctx.say("Channel unlocked").await?;
    log_note(&ctx, logging::GREEN, "Channel unlocked",
        vec![("Channel", format!("<#{}>", ctx.channel_id()), true)]).await;
    Ok(())
}

/// Account info and record count for a user
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn whois(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
) -> Result<(), Error> {
    let uid = user.id.get();
    let n = ctx.data().with_db(move |c| db::count(c, uid)).await?;
    let member = guild(&ctx)?.member(ctx, user.id).await.ok();

    let mut e = serenity::CreateEmbed::new()
        .title(user.tag())
        .thumbnail(user.face())
        .field("ID", user.id.to_string(), true)
        .field("Created", format!("<t:{}:R>", user.id.created_at().unix_timestamp()), true)
        .field("Records", n.to_string(), true);

    if let Some(m) = member {
        if let Some(joined) = m.joined_at {
            e = e.field("Joined", format!("<t:{}:R>", joined.unix_timestamp()), true);
        }
        if let Some(nick) = &m.nick {
            e = e.field("Nickname", nick.clone(), true);
        }
        let roles = m.roles.iter().map(|r| format!("<@&{r}>")).collect::<Vec<_>>().join(" ");
        if !roles.is_empty() {
            e = e.field("Roles", util::clamp_field(&roles), false);
        }
        if let Some(until) = m.communication_disabled_until {
            if until.unix_timestamp() > db::now() {
                e = e.field("Muted until", format!("<t:{}:R>", until.unix_timestamp()), true);
            }
        }
    } else {
        e = e.field("In server", "no", true);
    }

    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}

#[derive(Debug, poise::ChoiceParameter)]
pub enum Kind {
    Warn,
    Note,
    Mute,
    Kick,
    Ban,
    Softban,
    Invite,
    #[name = "image spam"]
    ImageSpam,
}

impl Kind {
    fn key(&self) -> &'static str {
        match self {
            Kind::Warn => "warn",
            Kind::Note => "note",
            Kind::Mute => "mute",
            Kind::Kick => "kick",
            Kind::Ban => "ban",
            Kind::Softban => "softban",
            Kind::Invite => "invite",
            Kind::ImageSpam => "imagespam",
        }
    }
}

/// Browse recent cases across everyone, newest first
#[poise::command(slash_command, guild_only, required_permissions = "MODERATE_MEMBERS")]
pub async fn cases(
    ctx: Ctx<'_>,
    #[description = "Only this kind of action"] kind: Option<Kind>,
    #[description = "Only cases opened by this moderator"] moderator: Option<serenity::User>,
    #[description = "Reason contains this text"] search: Option<String>,
    #[description = "How many to show (1-50, default 20)"]
    #[min = 1]
    #[max = 50]
    limit: Option<u8>,
) -> Result<(), Error> {
    let n = limit.unwrap_or(20) as i64;
    let k = kind.as_ref().map(|k| k.key().to_string());
    let m = moderator.as_ref().map(|u| u.id.get());
    let q = search.clone();

    let rows = ctx
        .data()
        .with_db(move |c| db::browse(c, k, m, q, n))
        .await?;

    if rows.is_empty() {
        ctx.say("No cases match that").await?;
        return Ok(());
    }

    let body: String = rows
        .iter()
        .map(|r| {
            format!(
                "`#{}` **{}** <@{}> - <t:{}:R> by <@{}>\n{}",
                r.id,
                r.kind,
                r.user_id,
                r.created,
                r.mod_id,
                show(&r.reason)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let filters = [
        kind.map(|k| format!("kind: {}", k.key())),
        moderator.map(|u| format!("by: {}", u.tag())),
        search.map(|s| format!("reason contains: {s}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" - ");

    let mut e = serenity::CreateEmbed::new()
        .title(format!("{} case(s)", rows.len()))
        .description(util::clamp_desc(&body));
    if !filters.is_empty() {
        e = e.footer(serenity::CreateEmbedFooter::new(filters));
    }
    ctx.send(poise::CreateReply::default().embed(e)).await?;
    Ok(())
}
