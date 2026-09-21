use crate::{cmds::log_note, db, logging, util, Ctx, Data, Error};
use poise::serenity_prelude as serenity;
use poise::serenity_prelude::PermissionOverwriteType as Ow;
use poise::ChoiceParameter;

const CATEGORY: u64 = 1551418015042768906;
const USER_ISSUE_ROLE: u64 = 1551007200577585242;
const BUG_ROLE: u64 = 1545893087031599145;

const PINGS_KEY: &str = "ticket_pings";
const CLOSE: &str = "ticket:close";
const CLOSE_REASON: &str = "ticket:close_reason";
const REASON_MODAL: &str = "ticket:reason";
const NOT_TICKET: &str = "This isn't a ticket channel";

const ACCESS: serenity::Permissions = serenity::Permissions::VIEW_CHANNEL
    .union(serenity::Permissions::SEND_MESSAGES)
    .union(serenity::Permissions::READ_MESSAGE_HISTORY)
    .union(serenity::Permissions::ATTACH_FILES)
    .union(serenity::Permissions::EMBED_LINKS);

const MOD: serenity::Permissions =
    serenity::Permissions::MODERATE_MEMBERS.union(serenity::Permissions::ADMINISTRATOR);

#[derive(Debug, Clone, Copy, PartialEq, poise::ChoiceParameter)]
pub enum Kind {
    #[name = "User issue"]
    UserIssue,
    #[name = "Bug report"]
    BugReport,
}

impl Kind {
    fn prefix(self) -> &'static str {
        match self {
            Kind::UserIssue => "userissue-",
            Kind::BugReport => "bug-",
        }
    }

    fn role(self) -> serenity::RoleId {
        serenity::RoleId::new(match self {
            Kind::UserIssue => USER_ISSUE_ROLE,
            Kind::BugReport => BUG_ROLE,
        })
    }

    fn from_channel(name: &str) -> Option<Kind> {
        [Kind::UserIssue, Kind::BugReport].into_iter().find(|k| {
            name.strip_prefix(k.prefix())
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
    }
}

fn opener_of(topic: &str) -> Option<u64> {
    topic.strip_prefix("Opened by <@")?.strip_suffix('>')?.parse().ok()
}

async fn ticket_of(
    ctx: &serenity::Context,
    id: serenity::ChannelId,
) -> Option<(serenity::GuildChannel, Kind)> {
    let ch = id.to_channel(ctx).await.ok()?.guild()?;
    if ch.parent_id != Some(serenity::ChannelId::new(CATEGORY)) {
        return None;
    }
    let kind = Kind::from_channel(&ch.name)?;
    Some((ch, kind))
}

async fn deny(ctx: Ctx<'_>, msg: &str) -> Result<(), Error> {
    ctx.send(poise::CreateReply::default().content(msg).ephemeral(true))
        .await?;
    Ok(())
}

fn perms_in(
    ctx: Ctx<'_>,
    ch: &serenity::GuildChannel,
    m: &serenity::Member,
) -> serenity::Permissions {
    ctx.guild()
        .map(|g| g.user_permissions_in(ch, m))
        .unwrap_or(serenity::Permissions::empty())
}

/// Support tickets
#[poise::command(
    slash_command,
    guild_only,
    subcommands("ticket_create", "ticket_add", "ticket_remove", "ticket_close", "ticket_pings"),
    subcommand_required
)]
pub async fn ticket(_: Ctx<'_>) -> Result<(), Error> {
    Ok(())
}

/// Open a ticket
#[poise::command(slash_command, rename = "create")]
pub async fn ticket_create(
    ctx: Ctx<'_>,
    #[description = "What the ticket is about"] category: Kind,
) -> Result<(), Error> {
    ctx.defer_ephemeral().await?;
    let guild = ctx.guild_id().ok_or("guild only")?;
    let author = ctx.author().id;
    let role = category.role();

    let key = format!("ticket_{}", category.prefix());
    let (n, pings) = ctx
        .data()
        .with_db(move |c| {
            let n = db::kv_u64(c, &key).unwrap_or(0) + 1;
            db::kv_set(c, &key, &n.to_string())?;
            Ok((n, db::kv_get(c, PINGS_KEY)?.as_deref() != Some("0")))
        })
        .await?;

    let none = serenity::Permissions::empty();
    let ow = |kind, allow, deny| serenity::PermissionOverwrite { allow, deny, kind };
    let ours = vec![
        ow(Ow::Role(serenity::RoleId::new(guild.get())), none, serenity::Permissions::VIEW_CHANNEL),
        ow(Ow::Role(role), ACCESS, none),
        ow(Ow::Member(author), ACCESS, none),
        ow(Ow::Member(ctx.framework().bot_id), ACCESS, none),
    ];
    let cat = serenity::ChannelId::new(CATEGORY)
        .to_channel(ctx)
        .await?
        .guild()
        .ok_or("ticket category is missing")?;
    let mut overwrites: Vec<_> = cat
        .permission_overwrites
        .into_iter()
        .filter(|o| !ours.iter().any(|x| x.kind == o.kind))
        .collect();
    overwrites.extend(ours);

    let name = format!("{}{n}", category.prefix());
    let ch = guild
        .create_channel(
            ctx.http(),
            serenity::CreateChannel::new(&name)
                .kind(serenity::ChannelType::Text)
                .category(serenity::ChannelId::new(CATEGORY))
                .topic(format!("Opened by <@{author}>"))
                .permissions(overwrites),
        )
        .await?;

    let mut content = format!("<@{author}>");
    let mut mentions = serenity::CreateAllowedMentions::new().users([author]);
    if pings {
        content.push_str(&format!(" <@&{role}>"));
        mentions = mentions.roles([role]);
    }
    let embed = serenity::CreateEmbed::new()
        .color(logging::GREEN)
        .description(format!(
            "<@{author}>\n{} created! Please wait for someone to be with you. \
             While you wait, state your reason for making the ticket.",
            category.name()
        ));
    let buttons = serenity::CreateActionRow::Buttons(vec![
        serenity::CreateButton::new(CLOSE)
            .label("Close")
            .style(serenity::ButtonStyle::Danger),
        serenity::CreateButton::new(CLOSE_REASON)
            .label("Close with Reason")
            .style(serenity::ButtonStyle::Secondary),
    ]);
    ch.id
        .send_message(
            ctx.http(),
            serenity::CreateMessage::new()
                .content(content)
                .embed(embed)
                .components(vec![buttons])
                .allowed_mentions(mentions),
        )
        .await?;

    ctx.say(format!("Opened <#{}>", ch.id)).await?;

    let e = serenity::CreateEmbed::new()
        .color(logging::GREEN)
        .title(format!("Ticket opened - {name}"))
        .field("User", format!("{} `{author}`", ctx.author().tag()), true)
        .field("Type", category.name(), true)
        .field("Channel", format!("<#{}>", ch.id), true)
        .timestamp(serenity::Timestamp::now());
    logging::send(ctx.serenity_context(), ctx.data(), e).await;
    Ok(())
}

/// Give a user access to this ticket
#[poise::command(slash_command, rename = "add", required_permissions = "MODERATE_MEMBERS")]
pub async fn ticket_add(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
) -> Result<(), Error> {
    let Some((ch, _)) = ticket_of(ctx.serenity_context(), ctx.channel_id()).await else {
        return deny(ctx, NOT_TICKET).await;
    };
    let Ok(member) = ch.guild_id.member(ctx, user.id).await else {
        return deny(ctx, "That user isn't in the server").await;
    };
    if perms_in(ctx, &ch, &member).view_channel() {
        return deny(ctx, "User already has access to this ticket!").await;
    }

    ch.id
        .create_permission(
            ctx.http(),
            serenity::PermissionOverwrite {
                allow: ACCESS,
                deny: serenity::Permissions::empty(),
                kind: Ow::Member(user.id),
            },
        )
        .await?;
    ctx.say(format!("Added <@{}>", user.id)).await?;

    log_note(
        &ctx,
        logging::GREEN,
        "Ticket access added",
        vec![
            ("User", format!("{} `{}`", user.tag(), user.id), true),
            ("Ticket", format!("<#{}>", ch.id), true),
        ],
    )
    .await;
    Ok(())
}

/// Take a user's access to this ticket away
#[poise::command(slash_command, rename = "remove", required_permissions = "MODERATE_MEMBERS")]
pub async fn ticket_remove(
    ctx: Ctx<'_>,
    #[description = "User"] user: serenity::User,
) -> Result<(), Error> {
    let Some((ch, _)) = ticket_of(ctx.serenity_context(), ctx.channel_id()).await else {
        return deny(ctx, NOT_TICKET).await;
    };
    if user.id == ctx.framework().bot_id {
        return deny(ctx, "Removing me would leave the ticket unclosable").await;
    }
    let Ok(member) = ch.guild_id.member(ctx, user.id).await else {
        return deny(ctx, "That user isn't in the server").await;
    };
    let perms = perms_in(ctx, &ch, &member);
    if !perms.view_channel() {
        return deny(ctx, "User already can't view this ticket").await;
    }
    if perms.administrator() {
        return deny(ctx, "Administrators see every channel, they can't be removed").await;
    }

    ch.id
        .create_permission(
            ctx.http(),
            serenity::PermissionOverwrite {
                allow: serenity::Permissions::empty(),
                deny: serenity::Permissions::VIEW_CHANNEL,
                kind: Ow::Member(user.id),
            },
        )
        .await?;
    ctx.say(format!("Removed {}", user.tag())).await?;

    log_note(
        &ctx,
        logging::AMBER,
        "Ticket access removed",
        vec![
            ("User", format!("{} `{}`", user.tag(), user.id), true),
            ("Ticket", format!("<#{}>", ch.id), true),
        ],
    )
    .await;
    Ok(())
}

/// Close this ticket
#[poise::command(slash_command, rename = "close", required_permissions = "MODERATE_MEMBERS")]
pub async fn ticket_close(ctx: Ctx<'_>) -> Result<(), Error> {
    let Some((ch, kind)) = ticket_of(ctx.serenity_context(), ctx.channel_id()).await else {
        return deny(ctx, NOT_TICKET).await;
    };
    ctx.say("Closing ticket").await?;
    close_ticket(ctx.serenity_context(), ctx.data(), ch, kind, ctx.author(), None).await
}

/// Turn staff role pings on new tickets on or off
#[poise::command(slash_command, rename = "pings", required_permissions = "MODERATE_MEMBERS")]
pub async fn ticket_pings(
    ctx: Ctx<'_>,
    #[description = "Ping staff roles when a ticket opens"] enabled: bool,
) -> Result<(), Error> {
    ctx.data()
        .with_db(move |c| db::kv_set(c, PINGS_KEY, if enabled { "1" } else { "0" }))
        .await?;
    let state = if enabled { "on" } else { "off" };
    ctx.say(format!("Ticket pings {state}")).await?;
    log_note(&ctx, logging::GREY, "Ticket pings", vec![("Pings", state.into(), true)]).await;
    Ok(())
}

async fn close_ticket(
    ctx: &serenity::Context,
    data: &Data,
    ch: serenity::GuildChannel,
    kind: Kind,
    closer: &serenity::User,
    reason: Option<String>,
) -> Result<(), Error> {
    let opener = ch.topic.as_deref().and_then(opener_of);
    let cid = ch.id.get();
    let mut people = data.with_db(move |c| db::participants(c, cid)).await?;
    if let Some(o) = opener.filter(|o| !people.contains(o)) {
        people.push(o);
    }

    ch.id.delete(&ctx.http).await?;

    let reason = reason
        .map(|r| util::clamp_reason(r.trim()))
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| "no reason specified".into());
    let list = people
        .iter()
        .map(|id| format!("<@{id}>"))
        .collect::<Vec<_>>()
        .join(" ");

    let embed = serenity::CreateEmbed::new()
        .color(logging::RED)
        .title(format!("Ticket closed - {}", ch.name))
        .field("Type", kind.name(), true)
        .field("Opened by", opener.map_or("unknown".to_string(), |o| format!("<@{o}>")), true)
        .field("Closed by", format!("{} `{}`", closer.tag(), closer.id), true)
        .field("Created", format!("<t:{}:F>", ch.id.created_at().unix_timestamp()), true)
        .field("Closed", format!("<t:{}:F>", db::now()), true)
        .field("Reason", util::clamp_field(&reason), false)
        .field("Participants", util::clamp_field(if list.is_empty() { "none" } else { &list }), false)
        .timestamp(serenity::Timestamp::now());

    let mut failed = 0;
    for id in &people {
        let dm = serenity::CreateMessage::new().embed(embed.clone());
        if serenity::UserId::new(*id).direct_message(ctx, dm).await.is_err() {
            failed += 1;
        }
    }

    let log = if failed > 0 {
        embed.field("DMs failed", failed.to_string(), true)
    } else {
        embed
    };
    logging::send(ctx, data, log).await;
    Ok(())
}

fn ephemeral(msg: &str) -> serenity::CreateInteractionResponse {
    serenity::CreateInteractionResponse::Message(
        serenity::CreateInteractionResponseMessage::new()
            .content(msg)
            .ephemeral(true),
    )
}

async fn gate(
    ctx: &serenity::Context,
    member: Option<&serenity::Member>,
    channel: serenity::ChannelId,
) -> Result<(serenity::GuildChannel, Kind), &'static str> {
    if !member.and_then(|m| m.permissions).is_some_and(|p| p.intersects(MOD)) {
        return Err("No access");
    }
    ticket_of(ctx, channel).await.ok_or(NOT_TICKET)
}

pub async fn on_interaction(
    ctx: &serenity::Context,
    interaction: &serenity::Interaction,
    data: &Data,
) -> Result<(), Error> {
    match interaction {
        serenity::Interaction::Component(c)
            if [CLOSE, CLOSE_REASON].contains(&c.data.custom_id.as_str()) =>
        {
            let (ch, kind) = match gate(ctx, c.member.as_ref(), c.channel_id).await {
                Ok(t) => t,
                Err(msg) => {
                    c.create_response(ctx, ephemeral(msg)).await?;
                    return Ok(());
                }
            };
            if c.data.custom_id == CLOSE_REASON {
                let input = serenity::CreateInputText::new(
                    serenity::InputTextStyle::Paragraph,
                    "Reason",
                    "reason",
                )
                .max_length(512);
                let modal = serenity::CreateModal::new(REASON_MODAL, "Close with reason")
                    .components(vec![serenity::CreateActionRow::InputText(input)]);
                c.create_response(ctx, serenity::CreateInteractionResponse::Modal(modal))
                    .await?;
                return Ok(());
            }
            c.create_response(ctx, serenity::CreateInteractionResponse::Acknowledge)
                .await?;
            close_ticket(ctx, data, ch, kind, &c.user, None).await
        }

        serenity::Interaction::Modal(m) if m.data.custom_id == REASON_MODAL => {
            let (ch, kind) = match gate(ctx, m.member.as_ref(), m.channel_id).await {
                Ok(t) => t,
                Err(msg) => {
                    m.create_response(ctx, ephemeral(msg)).await?;
                    return Ok(());
                }
            };
            let reason = m
                .data
                .components
                .iter()
                .flat_map(|row| &row.components)
                .find_map(|c| match c {
                    serenity::ActionRowComponent::InputText(t) => t.value.clone(),
                    _ => None,
                });
            m.create_response(ctx, serenity::CreateInteractionResponse::Acknowledge)
                .await?;
            close_ticket(ctx, data, ch, kind, &m.user, reason).await
        }

        _ => Ok(()),
    }
}
