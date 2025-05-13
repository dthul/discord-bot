use command_macro::command;
use lib::db;
use serenity::{all::Mentionable, futures::StreamExt};
use std::collections::HashMap;

#[command]
#[regex(r"list\s*players")]
#[level(host)]
#[help(
    "list players",
    "shows information about people in this channel and people signed up for this channel's events on Meetup."
)]
fn list_players<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let pool = context.pool();
    let mut tx = pool.begin().await?;
    // Check whether this is a bot controlled channel
    let event_series = lib::get_channel_series(context.msg.channel_id, &mut tx).await?;
    let channel_roles = lib::get_channel_roles(context.msg.channel_id, &mut tx).await?;
    let (event_series, channel_roles) = match (event_series, channel_roles) {
        (Some(series), Some(roles)) => (series, roles),
        _ => {
            context
                .msg
                .channel_id
                .say(&context.ctx.http, lib::strings::CHANNEL_NOT_BOT_CONTROLLED)
                .await
                .ok();
            return Ok(());
        }
    };
    // Get all members that RSVPd "yes" to the upcoming events (or the last event if there are no upcoming events)
    let upcoming_events = db::get_upcoming_events_for_series(&pool, event_series).await?;
    let events = if upcoming_events.len() > 0 {
        upcoming_events
    } else {
        let last_event = db::get_last_event_in_series(&pool, event_series).await?;
        match last_event {
            Some(event) => vec![event],
            None => {
                context
                    .msg
                    .channel_id
                    .say(&context.ctx.http, "There are no events")
                    .await
                    .ok();
                return Ok(());
            }
        }
    };
    let event_ids: Vec<_> = events.iter().map(|event| event.id).collect();
    // Now, look up all participants of those events
    let rsvpd_members = db::get_events_participants(&event_ids, /*hosts*/ false, &pool).await?;

    // Look up all Discord users that have the player role
    // TODO: check whether this returns offline members
    let discord_player_ids = if let Some(guild_id) = context.msg.guild_id {
        let mut discord_player_ids = vec![];
        let mut members = guild_id.members_iter(&context.ctx.http).boxed();
        while let Some(member_result) = members.next().await {
            if let Ok(member) = member_result {
                if member.roles.contains(&channel_roles.user) {
                    discord_player_ids.push(member.user.id);
                }
            }
        }
        discord_player_ids
    } else {
        return Ok(());
    };
    let channel_members = db::discord_ids_to_members(&discord_player_ids, &pool).await?;

    // Four categories of users:
    // - RSVPd member (with Discord ID) [The following Discord users signed up for this event (in channel? yes / no)]
    // - RSVPd member (without Discord ID) [The following people are signed up for this event but are not linked to a Discord user]
    // - Channel member (with Meetup ID) (except for the ones that already fall into the first category)
    //   [The following Discord users are in this channel but did not sign up for this event]
    // - Channel member (without Meetup ID) (except for the ones that already fall into the first category)
    //   If there is no RSVPd member without Discord ID, use text from third category, otherwise use:
    //   [The following Discord users are in this channel but are not linked to a Meetup account. I cannot
    //   tell whether they signed up for this event on Meetup or not]

    // Create lists for all four categories
    let mut rsvpd_member_with_discord_id = HashMap::new();
    let mut rsvpd_member_without_discord_id = Vec::new();
    let mut channel_member_with_meetup_id = HashMap::new();
    let mut channel_member_without_meetup_id = Vec::new();

    for member in rsvpd_members {
        match member {
            db::Member {
                id,
                discord_id: Some(discord_id),
                meetup_id,
                ..
            } => {
                rsvpd_member_with_discord_id.insert(id, (discord_id, meetup_id));
            }
            db::Member {
                discord_id: None,
                meetup_id: Some(meetup_id),
                ..
            } => rsvpd_member_without_discord_id.push(meetup_id),
            _ => {}
        }
    }

    for (discord_id, member) in channel_members {
        match member {
            None => {
                channel_member_without_meetup_id.push(discord_id);
            }
            Some(db::MemberWithDiscord {
                id,
                meetup_id: Some(meetup_id),
                discord_id,
                ..
            }) => {
                if rsvpd_member_with_discord_id.contains_key(&id) {
                    continue;
                }
                channel_member_with_meetup_id.insert(id, (discord_id, meetup_id));
            }
            Some(db::MemberWithDiscord {
                id,
                meetup_id: None,
                discord_id,
                ..
            }) => {
                if rsvpd_member_with_discord_id.contains_key(&id) {
                    continue;
                }
                channel_member_without_meetup_id.push(discord_id);
            }
        }
    }

    // Construct the answer
    let mut reply = "**Note:** *this data might be a few minutes out of date. It is refreshed \
                     several times per hour.*\n\n"
        .to_string();

    if !rsvpd_member_with_discord_id.is_empty() {
        reply += "Discord users signed up for an upcoming event:\n";
        for &(discord_id, meetup_id) in rsvpd_member_with_discord_id.values() {
            let is_in_channel = discord_player_ids.contains(&discord_id);
            // If the user is in the channel try to not use a mention
            // in order not to unnecessarily ping them
            let user_mention = if is_in_channel {
                match discord_id.to_user(&context.ctx).await {
                    Ok(user) => match user
                        .nick_in(&context.ctx, lib::discord::sync::ids::GUILD_ID)
                        .await
                    {
                        Some(nick) => nick,
                        None => user.name.to_string(),
                    },
                    Err(_) => discord_id.mention().to_string(),
                }
            } else {
                discord_id.mention().to_string()
            };
            if let Some(meetup_id) = meetup_id {
                reply +=
                    &format!("• {user_mention} (<https://www.meetup.com/members/{meetup_id}/>)\n");
            } else {
                reply += &format!("• {user_mention}\n");
            }
            if is_in_channel {
                reply += " (in this channel ✅)\n";
            } else {
                reply += " (not in this channel ❌)\n";
            }
        }
        reply += "\n\n";
    }

    if !rsvpd_member_without_discord_id.is_empty() {
        reply += ":warning: People signed up for an upcoming event but not linked \
            to a Discord user:\n";
        for &meetup_id in &rsvpd_member_without_discord_id {
            reply += &format!(
                "• :x: <https://www.meetup.com/members/{meetup_id}/>\n",
                meetup_id = meetup_id
            );
        }
        reply += "\n\n";
    }

    if !channel_member_with_meetup_id.is_empty()
        || (!channel_member_without_meetup_id.is_empty()
            && rsvpd_member_without_discord_id.is_empty())
    {
        reply += "Discord users in this channel but not signed up for an \
                  upcoming event:\n";
        for &(discord_id, meetup_id) in channel_member_with_meetup_id.values() {
            reply += &format!(
                "• {discord_mention} (<https://www.meetup.com/members/{meetup_id}/>)\n",
                discord_mention = discord_id.mention(),
                meetup_id = meetup_id
            );
        }
        if rsvpd_member_without_discord_id.is_empty() {
            for &discord_id in &channel_member_without_meetup_id {
                reply += &format!("• {}\n", discord_id.mention());
            }
        }
        reply += "\n\n";
    }

    if !channel_member_without_meetup_id.is_empty() && !rsvpd_member_without_discord_id.is_empty() {
        reply += "Discord users in this channel but not linked to a Meetup \
                  account. I cannot tell whether they signed up for an upcoming event \
                  or not:\n";
        for &discord_id in &channel_member_without_meetup_id {
            reply += &format!("• {}\n", discord_id.mention());
        }
    }
    const LIMIT: usize = serenity::constants::MESSAGE_CODE_LIMIT;
    // Split the reply if necessary
    let mut reply = reply.as_str();
    while reply.chars().count() > 0 {
        if let Some((idx, _c)) = reply.char_indices().skip(LIMIT / 2).next() {
            context
                .msg
                .channel_id
                .say(&context.ctx.http, &reply[..idx])
                .await
                .ok();
            reply = &reply[idx..];
        } else {
            // Send the rest of the message
            context
                .msg
                .channel_id
                .say(&context.ctx.http, reply)
                .await
                .ok();
            break;
        }
    }
    Ok(())
}
