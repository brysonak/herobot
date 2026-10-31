use poise::serenity_prelude as serenity;

const CHANNEL: u64 = 1545855300983193733;
const RULES: &str = "https://discord.com/channels/1545855299393429544/1558999981099327509";
const PLAY: &str = "https://discord.com/channels/1545855299393429544/1545892580112207912";

const BANNER: &[u8] = include_bytes!("../assets/herobanner.png");
const BANNER_NAME: &str = "herobanner.png";

pub async fn on_join(ctx: &serenity::Context, member: &serenity::Member) {
    if member.user.bot {
        return;
    }

    let embed = serenity::CreateEmbed::new()
        .description(format!(
            "Welcome <@{}> to herodrop! Please read {RULES} and head to {PLAY} to play!",
            member.user.id
        ))
        .image(format!("attachment://{BANNER_NAME}"));
    let msg = serenity::CreateMessage::new()
        .embed(embed)
        .add_file(serenity::CreateAttachment::bytes(BANNER, BANNER_NAME));

    if let Err(e) = serenity::ChannelId::new(CHANNEL)
        .send_message(&ctx.http, msg)
        .await
    {
        eprintln!("welcome message failed: {e}");
    }
}
