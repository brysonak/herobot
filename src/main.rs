mod cmds;
mod db;
mod logging;
mod purge;
mod spam;
mod tickets;
mod util;

use poise::serenity_prelude as serenity;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Ctx<'a> = poise::Context<'a, Data, Error>;

pub const CACHE_KEEP_DAYS: i64 = 14;

pub struct Data {
    pub db: spam::Db,
    pub log_channel: serenity::ChannelId,
    pub guild: serenity::GuildId,
    pub honeypot: spam::Slot,
    pub images: spam::Windows,
    pub unban_wake: Arc<Notify>,
}

impl Data {
    pub async fn with_db<T, F>(&self, f: F) -> Result<T, Error>
    where
        F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let db = self.db.clone();
        let out = tokio::task::spawn_blocking(move || {
            let c = db.lock().unwrap();
            f(&c)
        })
        .await?;
        Ok(out?)
    }
}

fn env_id(key: &str) -> u64 {
    std::env::var(key)
        .unwrap_or_else(|_| panic!("{key} is not set"))
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{key} must be a numeric Discord id"))
}

#[tokio::main]
async fn main() {
    let token = std::env::var("TOKEN").expect("TOKEN is not set");
    let log_channel = serenity::ChannelId::new(env_id("LOG_CHANNEL"));
    let guild = serenity::GuildId::new(env_id("GUILD_ID"));
    let db_path = std::env::var("DB_PATH").unwrap_or_else(|_| "herobot.db".into());

    let conn = db::open(&db_path).expect("could not open database");
    let db: spam::Db = Arc::new(Mutex::new(conn));

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: cmds::all(),
            event_handler: |ctx, event, _fw, data| Box::pin(logging::handle(ctx, event, data)),
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            Box::pin(async move {
                poise::builtins::register_in_guild(ctx, &framework.options().commands, guild)
                    .await?;
                println!(
                    "{} ready, {} commands",
                    ready.user.name,
                    framework.options().commands.len()
                );

                let honeypot: spam::Slot = Arc::new(Mutex::new(None));
                if let Err(e) = spam::ensure_channel(ctx, &db, guild, &honeypot).await {
                    eprintln!("honeypot setup failed: {e}");
                }

                let unban_wake = Arc::new(Notify::new());
                let images: spam::Windows = Arc::new(Mutex::new(HashMap::new()));

                spam::spawn_rotation(ctx.clone(), db.clone(), guild, honeypot.clone());
                spawn_unban_sweeper(ctx.http.clone(), db.clone(), guild, unban_wake.clone());
                spawn_janitor(db.clone(), images.clone());

                Ok(Data {
                    db,
                    log_channel,
                    guild,
                    honeypot,
                    images,
                    unban_wake,
                })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::GUILDS
        | serenity::GatewayIntents::GUILD_MEMBERS
        | serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT;

    let mut cache = serenity::Settings::default();
    cache.max_messages = 5_000;

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .cache_settings(cache)
        .await
        .expect("could not build client");

    if let Err(e) = client.start().await {
        eprintln!("client error: {e}");
    }
}

fn spawn_unban_sweeper(
    http: Arc<serenity::Http>,
    db: spam::Db,
    guild: serenity::GuildId,
    wake: Arc<Notify>,
) {
    tokio::spawn(async move {
        loop {
            let (due, next) = {
                let c = db.lock().unwrap();
                (
                    db::due_unbans(&c).unwrap_or_default(),
                    db::next_unban(&c).ok().flatten(),
                )
            };

            for (id, user) in due {
                let user = serenity::UserId::new(user);
                if let Err(e) = guild.unban(&http, user).await {
                    eprintln!("unban {user} failed: {e}");
                }
                if let Err(e) = db::mark_undone(&db.lock().unwrap(), id) {
                    eprintln!("could not mark record {id}: {e}");
                }
            }

            let wait = match next {
                Some(at) => (at - db::now()).clamp(1, 3600),
                None => 3600,
            };
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(wait as u64)) => {}
                _ = wake.notified() => {}
            }
        }
    });
}

fn spawn_janitor(db: spam::Db, images: spam::Windows) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tick.tick().await;
            let handle = db.clone();
            let _ = tokio::task::spawn_blocking(move || {
                db::prune_cache(&handle.lock().unwrap(), CACHE_KEEP_DAYS)
            })
            .await;

            let cutoff = db::now_ms() - spam::IMAGE_WINDOW_MS;
            let mut seen = images.lock().unwrap();
            seen.retain(|_, stamps| {
                stamps.retain(|t| *t > cutoff);
                !stamps.is_empty()
            });
        }
    });
}
