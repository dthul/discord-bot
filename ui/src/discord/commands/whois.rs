use command_macro::command;
use lib::db;
use serenity::{all::Mentionable, model::id::UserId, nonmax::NonMaxU64};

#[command]
#[regex(
    r"whois\s+(?:{mention_pattern}|{username_tag_pattern}|{username_pattern}|meetup\s+{meetup_id_pattern})",
    mention_pattern,
    username_tag_pattern,
    username_pattern,
    meetup_id_pattern
)]
#[level(admin)]
#[help(
    "whois `@some-discord-user`",
    "_(needs proper mention)_ shows the Meetup profile of the mentioned Discord user"
)]
#[help(
    "whois `some-discord-username`",
    "_(no mention)_ shows the Meetup profile of the mentioned Discord user"
)]
#[help(
    "whois meetup `meetup-ID`",
    "shows the Discord user linked to the provided Meetup profile"
)]
fn whois<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    if let Some(capture) = captures.name("mention_id") {
        // Look up by Discord ID
        let discord_id = capture.as_str();
        // Try to convert the specified ID to an integer
        let discord_id = match discord_id.parse::<NonMaxU64>() {
            Ok(id) => UserId::from(id.get()),
            _ => {
                context
                    .msg
                    .channel_id
                    .say(
                        &context.ctx.http,
                        lib::strings::CHANNEL_ADD_USER_INVALID_DISCORD,
                    )
                    .await
                    .ok();
                return Ok(());
            }
        };
        whois_by_discord_id(context, discord_id).await?;
    } else if let Some(capture) = captures.name("discord_username_tag") {
        let username_tag = capture.as_str();
        // Look up by Discord username and tag
        whois_by_discord_username_tag(context, &username_tag).await?;
    } else if let Some(capture) = captures.name("discord_username") {
        let username = capture.as_str();
        // Look up by Discord username and tag
        whois_by_discord_username_tag(context, &username).await?;
    } else if let Some(capture) = captures.name("meetup_user_id") {
        // Look up by Meetup ID
        let meetup_id = capture.as_str();
        // Try to convert the specified ID to an integer
        let meetup_id = match meetup_id.parse::<u64>() {
            Ok(id) => id,
            _ => {
                context
                    .msg
                    .channel_id
                    .say(
                        &context.ctx.http,
                        lib::strings::CHANNEL_ADD_USER_INVALID_DISCORD,
                    )
                    .await
                    .ok();
                return Ok(());
            }
        };
        whois_by_meetup_id(context, meetup_id).await?
    }
    Ok(())
}

async fn whois_by_discord_id(
    context: &mut super::CommandContext,
    user_id: UserId,
) -> Result<(), lib::meetup::Error> {
    let pool = context.pool();
    let member = db::discord_ids_to_members(&[user_id], &pool).await?;
    match member.as_slice() {
        [(
            _,
            Some(db::MemberWithDiscord {
                meetup_id: Some(meetup_id),
                discord_nick,
                ..
            }),
        )] => {
            let message = if let Some(discord_nick) = discord_nick {
                format!(
                    "{} ({}) is linked to https://www.meetup.com/members/{}/",
                    user_id.mention(),
                    discord_nick,
                    meetup_id
                )
            } else {
                format!(
                    "{} is linked to https://www.meetup.com/members/{}/",
                    user_id.mention(),
                    meetup_id
                )
            };
            context
                .msg
                .channel_id
                .say(&context.ctx.http, message)
                .await
                .ok();
        }
        _ => {
            context
                .msg
                .channel_id
                .say(
                    &context.ctx.http,
                    format!(
                        "{} does not seem to be linked to a Meetup account",
                        user_id.mention()
                    ),
                )
                .await
                .ok();
        }
    }
    Ok(())
}

async fn whois_by_discord_username_tag(
    context: &mut super::CommandContext,
    username_tag: &str,
) -> Result<(), lib::meetup::Error> {
    let discord_id = lib::discord::sync::ids::GUILD_ID
        .to_guild_cached(&context.ctx.cache)
        .map(|guild| guild.member_named(username_tag).map(|m| m.user.id));
    let discord_id = if let Some(discord_id) = discord_id {
        discord_id
    } else {
        context
            .msg
            .channel_id
            .say(&context.ctx.http, "Something went wrong (guild not found)")
            .await
            .ok();
        return Ok(());
    };
    if let Some(discord_id) = discord_id {
        // Look up by Discord ID
        whois_by_discord_id(context, discord_id).await?;
    } else {
        context
            .msg
            .channel_id
            .say(
                &context.ctx.http,
                format!("{} is not a Discord user", username_tag),
            )
            .await
            .ok();
    }
    Ok(())
}

async fn whois_by_meetup_id(
    context: &mut super::CommandContext,
    meetup_id: u64,
) -> Result<(), lib::meetup::Error> {
    let pool = context.pool();
    let member = db::meetup_ids_to_members(&[meetup_id], &pool).await?;
    match member.as_slice() {
        [(
            _,
            Some(db::MemberWithMeetup {
                discord_id: Some(discord_id),
                discord_nick,
                ..
            }),
        )] => {
            let message = if let Some(discord_nick) = discord_nick {
                format!(
                    "https://www.meetup.com/members/{}/ is linked to {} ({})",
                    meetup_id,
                    discord_id.mention(),
                    discord_nick
                )
            } else {
                format!(
                    "https://www.meetup.com/members/{}/ is linked to {}",
                    meetup_id,
                    discord_id.mention()
                )
            };
            context
                .msg
                .channel_id
                .say(&context.ctx.http, message)
                .await
                .ok();
        }
        _ => {
            context
                .msg
                .channel_id
                .say(
                    &context.ctx.http,
                    format!(
                        "https://www.meetup.com/members/{}/ does not seem to be linked to a \
                         Discord user",
                        meetup_id
                    ),
                )
                .await
                .ok();
        }
    }
    Ok(())
}
