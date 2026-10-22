use super::server::Hook;
use crate::{logging, util, Error};
use poise::serenity_prelude as serenity;
use serde_json::Value;

const THANKS_CHANNEL: u64 = 1548069053946527754;

const ALL_SUBSCRIBERS_ROLE: u64 = 1551378471228280902;

struct Tier {
    id: &'static str,
    name: &'static str,
    role: u64,
}

const TIER_ROLES: &[Tier] = &[
    Tier {
        id: "",
        name: "Supporter",
        role: 1551297874992697464,
    },
    Tier {
        id: "",
        name: "Founder",
        role: 1551298223405006870,
    },
    Tier {
        id: "",
        name: "Hero",
        role: 1551298007624974366,
    },
    Tier {
        id: "",
        name: "Legend",
        role: 1551298387968655371,
    },
    Tier {
        id: "",
        name: "Show Off",
        role: 1551298457606688828,
    },
    Tier {
        id: "",
        name: "Main Character",
        role: 1551298457606688828,
    },
];

fn pick(table: &[Tier], tier_ids: &[&str]) -> Vec<u64> {
    let mut roles = vec![ALL_SUBSCRIBERS_ROLE];
    roles.extend(
        table
            .iter()
            .filter(|t| !t.id.is_empty() && tier_ids.contains(&t.id))
            .map(|t| t.role),
    );
    roles.sort_unstable();
    roles.dedup();
    roles
}

fn tier_name(id: &str, included: &[Value]) -> String {
    if let Some(t) = TIER_ROLES.iter().find(|t| !t.id.is_empty() && t.id == id) {
        return t.name.to_string();
    }
    included
        .iter()
        .find(|i| i["type"] == "tier" && i["id"] == id)
        .and_then(|i| i["attributes"]["title"].as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("tier {id}"))
}

pub async fn subscribed(h: &Hook, body: &Value) -> Result<(), Error> {
    let member = &body["data"];
    let included: &[Value] = body["included"].as_array().map_or(&[], |v| v.as_slice());

    let tier_ids: Vec<&str> = member["relationships"]["currently_entitled_tiers"]["data"]
        .as_array()
        .map(|a| a.iter().filter_map(|t| t["id"].as_str()).collect())
        .unwrap_or_default();
    let tiers = if tier_ids.is_empty() {
        "an unknown tier".to_string()
    } else {
        tier_ids
            .iter()
            .map(|id| tier_name(id, included))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let name = member["attributes"]["full_name"]
        .as_str()
        .unwrap_or("Someone");
    let discord = included
        .iter()
        .find(|i| i["type"] == "user")
        .and_then(|u| u["attributes"]["social_connections"]["discord"]["user_id"].as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n != 0)
        .map(serenity::UserId::new);

    let mut given = 0;
    let mut failed = 0;
    if let Some(uid) = discord {
        for role in pick(TIER_ROLES, &tier_ids) {
            let role = serenity::RoleId::new(role);
            match h
                .http
                .add_member_role(h.guild, uid, role, Some("Patreon subscription"))
                .await
            {
                Ok(_) => given += 1,
                Err(e) => {
                    failed += 1;
                    eprintln!("patreon: could not give {role} to {uid}: {e}");
                }
            }
        }
    }

    let who = match discord {
        Some(u) => format!("<@{u}>"),
        None => name.to_string(),
    };
    let mut mentions = serenity::CreateAllowedMentions::new();
    if let Some(u) = discord {
        mentions = mentions.users([u]);
    }
    let thanked = serenity::ChannelId::new(THANKS_CHANNEL)
        .send_message(
            &h.http,
            serenity::CreateMessage::new()
                .content(format!("Thank you {who} for subscribing with {tiers}!"))
                .allowed_mentions(mentions),
        )
        .await;

    let who_log = match discord {
        Some(u) => format!("<@{u}> `{u}`"),
        None => format!("{name} (no Discord linked)"),
    };
    let mut e = serenity::CreateEmbed::new()
        .color(logging::GREEN)
        .title("Patreon subscription")
        .description(util::clamp_desc(&format!(
            "{who_log} subscribed with {tiers}"
        )))
        .field("Roles given", given.to_string(), true)
        .timestamp(serenity::Timestamp::now());
    if failed > 0 {
        e = e.field("Role failures", failed.to_string(), true);
    }
    logging::send_to(&h.http, h.log_channel, e).await;

    thanked?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_roles() {
        let table = [
            Tier {
                id: "1",
                name: "a",
                role: 10,
            },
            Tier {
                id: "2",
                name: "b",
                role: 20,
            },
            Tier {
                id: "3",
                name: "c",
                role: 20,
            },
            Tier {
                id: "",
                name: "unset",
                role: 30,
            },
        ];
        let all = ALL_SUBSCRIBERS_ROLE;
        let sorted = |mut v: Vec<u64>| {
            v.sort_unstable();
            v
        };

        assert_eq!(pick(&table, &[]), vec![all]);
        assert_eq!(pick(&table, &["99"]), vec![all]);
        assert_eq!(pick(&table, &[""]), vec![all]);
        assert_eq!(pick(&table, &["1"]), sorted(vec![all, 10]));
        assert_eq!(pick(&table, &["2", "3"]), sorted(vec![all, 20]));
        assert_eq!(pick(&table, &["1", "2"]), sorted(vec![all, 10, 20]));
    }
}
