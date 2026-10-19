use crate::{util, Ctx, Error};
use poise::serenity_prelude as serenity;
use serde_json::{json, Value};
use std::time::Duration;

const MAX_CHARS: usize = 4000;
const TARGET: &str = "EN-US";

pub struct Deepl {
    client: reqwest::Client,
    url: String,
    key: String,
}

impl Deepl {
    pub fn from_env() -> Option<Self> {
        let key = std::env::var("DEEPL_KEY").ok()?.trim().to_string();
        if key.is_empty() {
            return None;
        }
        let host = if key.ends_with(":fx") {
            "api-free"
        } else {
            "api"
        };
        Some(Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .ok()?,
            url: format!("https://{host}.deepl.com/v2/translate"),
            key,
        })
    }

    async fn to_english(&self, text: &str) -> Result<(String, String), Error> {
        let resp = self
            .client
            .post(&self.url)
            .header("Authorization", format!("DeepL-Auth-Key {}", self.key))
            .json(&json!({ "text": [text], "target_lang": TARGET }))
            .send()
            .await?;

        match resp.status().as_u16() {
            200 => {}
            429 => return Err("DeepL is rate limiting me, try again shortly".into()),
            456 => return Err("DeepL quota is used up for this month".into()),
            s => return Err(format!("DeepL error {s}").into()),
        }

        let v: Value = resp.json().await?;
        let t = &v["translations"][0];
        let text = t["text"].as_str().ok_or("unexpected DeepL response")?;
        let from = t["detected_source_language"].as_str().unwrap_or("?");
        Ok((from.to_string(), text.to_string()))
    }
}

#[poise::command(prefix_command, guild_only, user_cooldown = 5)]
pub async fn translate(ctx: Ctx<'_>) -> Result<(), Error> {
    let Some(deepl) = &ctx.data().deepl else {
        return Err("translation isn't configured".into());
    };
    let Ctx::Prefix(p) = ctx else {
        return Ok(());
    };
    let Some(replied) = &p.msg.referenced_message else {
        return Err("reply to the message you want translated".into());
    };
    if replied.content.trim().is_empty() {
        return Err("that message has no text".into());
    }
    if replied.content.chars().count() > MAX_CHARS {
        return Err(format!("too long, the limit is {MAX_CHARS} characters").into());
    }

    ctx.defer().await?;
    let (from, out) = deepl.to_english(&replied.content).await?;

    let e = serenity::CreateEmbed::new()
        .description(util::clamp_desc(&out))
        .footer(serenity::CreateEmbedFooter::new(format!("from {from}")));
    ctx.send(poise::CreateReply::default().embed(e).reply(true))
        .await?;
    Ok(())
}