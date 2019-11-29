use futures_util::{compat::Future01CompatExt, lock::Mutex as AsyncMutex};
use lib::strings;
use redis::{Commands, PipelineCommands};
use regex::Regex;
use serenity::{
    model::{channel::Message, id::UserId, user::User},
    prelude::*,
};
use simple_error::SimpleError;
use std::{borrow::Cow, sync::Arc};

const MENTION_PATTERN: &'static str = r"(?:<@!?(?P<mention_id>[0-9]+)>)";
const USERNAME_TAG_PATTERN: &'static str = r"(?P<discord_username_tag>[^@#:]{2,32}#[0-9]+)";
const MEETUP_ID_PATTERN: &'static str = r"(?P<meetup_user_id>[0-9]+)";

pub struct Regexes {
    pub bot_mention: Regex,
    pub link_meetup_dm: Regex,
    pub link_meetup_mention: Regex,
    pub link_meetup_bot_admin_dm: Regex,
    pub link_meetup_bot_admin_mention: Regex,
    pub unlink_meetup_dm: Regex,
    pub unlink_meetup_mention: Regex,
    pub unlink_meetup_bot_admin_dm: Regex,
    pub unlink_meetup_bot_admin_mention: Regex,
    pub sync_meetup_mention: Regex,
    pub sync_discord_mention: Regex,
    pub add_user_bot_admin_mention: Regex,
    pub add_host_bot_admin_mention: Regex,
    pub remove_user_mention: Regex,
    pub remove_host_bot_admin_mention: Regex,
    pub stop_bot_admin_dm: Regex,
    pub stop_bot_admin_mention: Regex,
    pub send_expiration_reminder_bot_admin_mention: Regex,
    pub end_adventure_host_mention: Regex,
    pub help_dm: Regex,
    pub help_mention: Regex,
    pub refresh_user_token_admin_dm: Regex,
    pub rsvp_user_admin_mention: Regex,
    pub clone_event_admin_mention: Regex,
    pub schedule_session_mention: Regex,
    pub whois_bot_admin_dm: Regex,
    pub whois_bot_admin_mention: Regex,
}

impl Regexes {
    pub fn link_meetup(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.link_meetup_dm
        } else {
            &self.link_meetup_mention
        }
    }

    pub fn link_meetup_bot_admin(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.link_meetup_bot_admin_dm
        } else {
            &self.link_meetup_bot_admin_mention
        }
    }

    pub fn unlink_meetup(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.unlink_meetup_dm
        } else {
            &self.unlink_meetup_mention
        }
    }

    pub fn unlink_meetup_bot_admin(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.unlink_meetup_bot_admin_dm
        } else {
            &self.unlink_meetup_bot_admin_mention
        }
    }

    pub fn stop_bot_admin(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.stop_bot_admin_dm
        } else {
            &self.stop_bot_admin_mention
        }
    }

    pub fn whois_bot_admin(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.whois_bot_admin_dm
        } else {
            &self.whois_bot_admin_mention
        }
    }

    pub fn help(&self, is_dm: bool) -> &Regex {
        if is_dm {
            &self.help_dm
        } else {
            &self.help_mention
        }
    }
}

pub fn compile_regexes(bot_id: u64, bot_name: &str) -> Regexes {
    let bot_mention = format!(
        r"(?:<@{bot_id}>|@{bot_name})",
        bot_id = bot_id,
        bot_name = bot_name
    );
    let link_meetup_dm = r"^(?i)link[ -]?meetup\s*$";
    let link_meetup_mention = format!(
        r"^{bot_mention}\s+(?i)link[ -]?meetup\s*$",
        bot_mention = bot_mention
    );
    let link_meetup_bot_admin = format!(
        r"(?i)link[ -]?meetup\s+{mention_pattern}\s+(?P<meetupid>[0-9]+)",
        mention_pattern = MENTION_PATTERN
    );
    let link_meetup_bot_admin_dm = format!(
        r"^{link_meetup_bot_admin}\s*$",
        link_meetup_bot_admin = link_meetup_bot_admin
    );
    let link_meetup_bot_admin_mention = format!(
        r"^{bot_mention}\s+{link_meetup_bot_admin}\s*$",
        bot_mention = bot_mention,
        link_meetup_bot_admin = link_meetup_bot_admin
    );
    let unlink_meetup = r"(?i)unlink[ -]?meetup";
    let unlink_meetup_dm = format!(r"^{unlink_meetup}\s*$", unlink_meetup = unlink_meetup);
    let unlink_meetup_mention = format!(
        r"^{bot_mention}\s+{unlink_meetup}\s*$",
        bot_mention = bot_mention,
        unlink_meetup = unlink_meetup
    );
    let unlink_meetup_bot_admin = format!(
        r"(?i)unlink[ -]?meetup\s+{mention_pattern}",
        mention_pattern = MENTION_PATTERN
    );
    let unlink_meetup_bot_admin_dm = format!(
        r"^{unlink_meetup_bot_admin}\s*$",
        unlink_meetup_bot_admin = unlink_meetup_bot_admin
    );
    let unlink_meetup_bot_admin_mention = format!(
        r"^{bot_mention}\s+{unlink_meetup_bot_admin}\s*$",
        bot_mention = bot_mention,
        unlink_meetup_bot_admin = unlink_meetup_bot_admin
    );
    let sync_meetup_mention = format!(
        r"^{bot_mention}\s+(?i)sync\s+meetup\s*$",
        bot_mention = bot_mention
    );
    let sync_discord_mention = format!(
        r"^{bot_mention}\s+(?i)sync\s+discord\s*$",
        bot_mention = bot_mention
    );
    let add_user_bot_admin_mention = format!(
        r"^{bot_mention}\s+(?i)add\s+{mention_pattern}\s*$",
        bot_mention = bot_mention,
        mention_pattern = MENTION_PATTERN,
    );
    let add_host_bot_admin_mention = format!(
        r"^{bot_mention}\s+(?i)add\s+host\s+{mention_pattern}\s*$",
        bot_mention = bot_mention,
        mention_pattern = MENTION_PATTERN,
    );
    let remove_user_mention = format!(
        r"^{bot_mention}\s+(?i)remove\s+{mention_pattern}\s*$",
        bot_mention = bot_mention,
        mention_pattern = MENTION_PATTERN,
    );
    let remove_host_bot_admin_mention = format!(
        r"^{bot_mention}\s+(?i)remove\s+host\s+{mention_pattern}\s*$",
        bot_mention = bot_mention,
        mention_pattern = MENTION_PATTERN,
    );
    let stop_bot_admin_dm = r"^(?i)stop\s*$";
    let stop_bot_admin_mention =
        format!(r"^{bot_mention}\s+(?i)stop\s*$", bot_mention = bot_mention);
    let send_expiration_reminder_bot_admin_mention = format!(
        r"^{bot_mention}\s+(?i)remind\s+expiration\s*$",
        bot_mention = bot_mention
    );
    let end_adventure_host_mention = format!(
        r"^{bot_mention}\s+(?i)end\s+adventure\s*$",
        bot_mention = bot_mention
    );
    let help_dm = r"^(?i)help\s*$";
    let help_mention = format!(r"^{bot_mention}\s+(?i)help\s*$", bot_mention = bot_mention);
    let refresh_user_token_admin_dm = format!(
        r"^{bot_mention}\s+(?i)refresh\s+meetup(-|\s*)token\s+{mention_pattern}\s*$",
        bot_mention = bot_mention,
        mention_pattern = MENTION_PATTERN
    );
    let rsvp_user_admin_mention = format!(
        r"^{bot_mention}\s+(?i)rsvp\s+{mention_pattern}\s+(?P<meetup_event_id>[^\s]+)\s*$",
        bot_mention = bot_mention,
        mention_pattern = MENTION_PATTERN,
    );
    let clone_event_admin_mention = format!(
        r"^{bot_mention}\s+(?i)clone\s+event\s+(?P<meetup_event_id>[^\s]+)\s*$",
        bot_mention = bot_mention,
    );
    let schedule_session_mention = format!(
        r"^{bot_mention}\s+(?i)schedule\s+session\s*$",
        bot_mention = bot_mention,
    );
    let whois_bot_admin = format!(
        r"(?i)whois\s+{mention_pattern}|{username_tag_pattern}|{meetup_id_pattern}",
        mention_pattern = MENTION_PATTERN,
        username_tag_pattern = USERNAME_TAG_PATTERN,
        meetup_id_pattern = MEETUP_ID_PATTERN,
    );
    let whois_bot_admin_dm = format!(r"^{whois_bot_admin}\s*$", whois_bot_admin = whois_bot_admin);
    let whois_bot_admin_mention = format!(
        r"^{bot_mention}\s+{whois_bot_admin}\s*$",
        bot_mention = bot_mention,
        whois_bot_admin = whois_bot_admin
    );
    Regexes {
        bot_mention: Regex::new(&format!("^{}", bot_mention)).unwrap(),
        link_meetup_dm: Regex::new(link_meetup_dm).unwrap(),
        link_meetup_mention: Regex::new(link_meetup_mention.as_str()).unwrap(),
        link_meetup_bot_admin_dm: Regex::new(link_meetup_bot_admin_dm.as_str()).unwrap(),
        link_meetup_bot_admin_mention: Regex::new(link_meetup_bot_admin_mention.as_str()).unwrap(),
        unlink_meetup_dm: Regex::new(unlink_meetup_dm.as_str()).unwrap(),
        unlink_meetup_mention: Regex::new(unlink_meetup_mention.as_str()).unwrap(),
        unlink_meetup_bot_admin_dm: Regex::new(unlink_meetup_bot_admin_dm.as_str()).unwrap(),
        unlink_meetup_bot_admin_mention: Regex::new(unlink_meetup_bot_admin_mention.as_str())
            .unwrap(),
        sync_meetup_mention: Regex::new(sync_meetup_mention.as_str()).unwrap(),
        sync_discord_mention: Regex::new(sync_discord_mention.as_str()).unwrap(),
        add_user_bot_admin_mention: Regex::new(add_user_bot_admin_mention.as_str()).unwrap(),
        add_host_bot_admin_mention: Regex::new(add_host_bot_admin_mention.as_str()).unwrap(),
        remove_user_mention: Regex::new(remove_user_mention.as_str()).unwrap(),
        remove_host_bot_admin_mention: Regex::new(remove_host_bot_admin_mention.as_str()).unwrap(),
        stop_bot_admin_dm: Regex::new(stop_bot_admin_dm).unwrap(),
        stop_bot_admin_mention: Regex::new(stop_bot_admin_mention.as_str()).unwrap(),
        send_expiration_reminder_bot_admin_mention: Regex::new(
            send_expiration_reminder_bot_admin_mention.as_str(),
        )
        .unwrap(),
        end_adventure_host_mention: Regex::new(end_adventure_host_mention.as_str()).unwrap(),
        help_dm: Regex::new(help_dm).unwrap(),
        help_mention: Regex::new(help_mention.as_str()).unwrap(),
        refresh_user_token_admin_dm: Regex::new(refresh_user_token_admin_dm.as_str()).unwrap(),
        rsvp_user_admin_mention: Regex::new(rsvp_user_admin_mention.as_str()).unwrap(),
        clone_event_admin_mention: Regex::new(clone_event_admin_mention.as_str()).unwrap(),
        schedule_session_mention: Regex::new(schedule_session_mention.as_str()).unwrap(),
        whois_bot_admin_dm: Regex::new(whois_bot_admin_dm.as_str()).unwrap(),
        whois_bot_admin_mention: Regex::new(whois_bot_admin_mention.as_str()).unwrap(),
    }
}

impl super::bot::Handler {
    pub fn link_meetup(
        ctx: &Context,
        msg: &Message,
        user_id: u64,
    ) -> Result<(), lib::meetup::Error> {
        let redis_key_d2m = format!("discord_user:{}:meetup_user", user_id);
        let (redis_client, redis_connection_mutex, meetup_client_mutex, bot_id) = {
            let data = ctx.data.read();
            (
                data.get::<super::bot::RedisClientKey>()
                    .ok_or_else(|| SimpleError::new("Redis client was not set"))?
                    .clone(),
                data.get::<super::bot::RedisConnectionKey>()
                    .ok_or_else(|| SimpleError::new("Redis connection was not set"))?
                    .clone(),
                data.get::<super::bot::AsyncMeetupClientKey>()
                    .ok_or_else(|| SimpleError::new("Meetup client was not set"))?
                    .clone(),
                data.get::<super::bot::BotIdKey>()
                    .ok_or_else(|| SimpleError::new("Bot ID was not set"))?
                    .clone(),
            )
        };
        // Check if there is already a meetup id linked to this user
        // and issue a warning
        let linked_meetup_id: Option<u64> = {
            let mut redis_connection = redis_connection_mutex.lock();
            redis_connection.get(&redis_key_d2m)?
        };
        if let Some(linked_meetup_id) = linked_meetup_id {
            // Return value of the async block:
            // None = Meetup API unavailable
            // Some(None) = Meetup API available but no user found
            // Some(Some(user)) = User found
            let meetup_user = lib::ASYNC_RUNTIME.block_on(async {
                let meetup_client = meetup_client_mutex.lock().await.clone();
                match meetup_client {
                    None => Ok::<_, lib::meetup::Error>(None),
                    Some(meetup_client) => match meetup_client
                        .get_member_profile(Some(linked_meetup_id))
                        .await?
                    {
                        None => Ok(Some(None)),
                        Some(user) => Ok(Some(Some(user))),
                    },
                }
            })?;
            match meetup_user {
                Some(meetup_user) => match meetup_user {
                    Some(meetup_user) => {
                        let _ = msg.author.direct_message(ctx, |message| {
                            message.content(strings::DISCORD_ALREADY_LINKED_MESSAGE1(
                                &meetup_user.name,
                                bot_id.0,
                            ))
                        });
                        let _ = msg.react(ctx, "\u{2705}");
                    }
                    _ => {
                        let _ = msg.author.direct_message(ctx, |message| {
                            message.content(strings::NONEXISTENT_MEETUP_LINKED_MESSAGE(bot_id.0))
                        });
                        let _ = msg.react(ctx, "\u{2705}");
                    }
                },
                _ => {
                    let _ = msg.author.direct_message(ctx, |message| {
                        message.content(strings::DISCORD_ALREADY_LINKED_MESSAGE2(bot_id.0))
                    });
                    let _ = msg.react(ctx, "\u{2705}");
                }
            }
            return Ok(());
        }
        // TODO: creates a new Redis connection. Not optimal...
        let (_, url) = lib::ASYNC_RUNTIME.block_on(async {
            let redis_connection = redis_client.get_async_connection().compat().await?;
            lib::meetup::oauth2::generate_meetup_linking_link(redis_connection, user_id).await
        })?;
        let dm = msg.author.direct_message(ctx, |message| {
            message.content(strings::MEETUP_LINKING_MESSAGE(&url))
        });
        match dm {
            Ok(_) => {
                let _ = msg.react(ctx, "\u{2705}");
            }
            Err(why) => {
                eprintln!("Error sending Meetup linking DM: {:?}", why);
                let _ = msg.reply(ctx, "There was an error trying to send you instructions.");
            }
        }
        Ok(())
    }

    pub fn link_meetup_bot_admin(
        ctx: &Context,
        msg: &Message,
        regexes: &Regexes,
        user_id: u64,
        meetup_id: u64,
    ) -> Result<(), lib::meetup::Error> {
        let redis_key_d2m = format!("discord_user:{}:meetup_user", user_id);
        let redis_key_m2d = format!("meetup_user:{}:discord_user", meetup_id);
        let (redis_connection_mutex, meetup_client_mutex) = {
            let data = ctx.data.read();
            (
                data.get::<super::bot::RedisConnectionKey>()
                    .ok_or_else(|| SimpleError::new("Redis connection was not set"))?
                    .clone(),
                data.get::<super::bot::AsyncMeetupClientKey>()
                    .ok_or_else(|| SimpleError::new("Meetup client was not set"))?
                    .clone(),
            )
        };
        // Check if there is already a meetup id linked to this user
        // and issue a warning
        let linked_meetup_id: Option<u64> = {
            let mut redis_connection = redis_connection_mutex.lock();
            redis_connection.get(&redis_key_d2m)?
        };
        if let Some(linked_meetup_id) = linked_meetup_id {
            if linked_meetup_id == meetup_id {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!(
                        "All good, this Meetup account was already linked to <@{}>",
                        user_id
                    ),
                );
                return Ok(());
            } else {
                // TODO: answer in DM?
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!(
                        "<@{discord_id}> is already linked to a different Meetup account. If you \
                         want to change this, unlink the currently linked Meetup account first by \
                         writing:\n{bot_mention} unlink meetup <@{discord_id}>",
                        discord_id = user_id,
                        bot_mention = regexes.bot_mention
                    ),
                );
                return Ok(());
            }
        }
        // Check if this meetup id is already linked
        // and issue a warning
        let linked_discord_id: Option<u64> = {
            let mut redis_connection = redis_connection_mutex.lock();
            redis_connection.get(&redis_key_m2d)?
        };
        if let Some(linked_discord_id) = linked_discord_id {
            let _ = msg.author.direct_message(ctx, |message_builder| {
                message_builder.content(format!(
                    "This Meetup account is alread linked to <@{linked_discord_id}>. If you want \
                     to change this, unlink the Meetup account first by writing\n{bot_mention} \
                     unlink meetup <@{linked_discord_id}>",
                    linked_discord_id = linked_discord_id,
                    bot_mention = regexes.bot_mention
                ))
            });
            return Ok(());
        }
        // The user has not yet linked their meetup account.
        // Test whether the specified Meetup user actually exists.
        let meetup_user = lib::ASYNC_RUNTIME.block_on(async {
            let meetup_client = meetup_client_mutex.lock().await.clone();
            match meetup_client {
                None => {
                    return Err(lib::meetup::Error::from(SimpleError::new(
                        "Meetup API unavailable",
                    )))
                }
                Some(meetup_client) => meetup_client
                    .get_member_profile(Some(meetup_id))
                    .await
                    .map_err(Into::into),
            }
        })?;
        match meetup_user {
            None => {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    "It looks like this Meetup profile does not exist",
                );
                return Ok(());
            }
            Some(meetup_user) => {
                let mut successful = false;
                {
                    let mut redis_connection = redis_connection_mutex.lock();
                    // Try to atomically set the meetup id
                    redis::transaction(
                        &mut *redis_connection,
                        &[&redis_key_d2m, &redis_key_m2d],
                        |con, pipe| {
                            let linked_meetup_id: Option<u64> = con.get(&redis_key_d2m)?;
                            let linked_discord_id: Option<u64> = con.get(&redis_key_m2d)?;
                            if linked_meetup_id.is_some() || linked_discord_id.is_some() {
                                // The meetup id was linked in the meantime, abort
                                successful = false;
                                // Execute empty transaction just to get out of the closure
                                pipe.query(con)
                            } else {
                                pipe.sadd("meetup_users", meetup_id)
                                    .sadd("discord_users", user_id)
                                    .set(&redis_key_d2m, meetup_id)
                                    .set(&redis_key_m2d, user_id);
                                successful = true;
                                pipe.query(con)
                            }
                        },
                    )?;
                }
                if successful {
                    let photo_url = meetup_user.photo.as_ref().map(|p| p.thumb_link.as_str());
                    let _ = msg.channel_id.send_message(&ctx.http, |message| {
                        message.embed(|embed| {
                            embed.title("Linked Meetup account");
                            embed.description(format!(
                                "Successfully linked <@{}> to {}'s Meetup account",
                                user_id, meetup_user.name
                            ));
                            if let Some(photo_url) = photo_url {
                                embed.image(photo_url)
                            } else {
                                embed
                            }
                        })
                    });
                    return Ok(());
                } else {
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, "Could not assign meetup id (timing error)");
                    return Ok(());
                }
            }
        }
    }

    pub fn unlink_meetup(
        ctx: &Context,
        msg: &Message,
        is_bot_admin_command: bool,
        user_id: u64,
        bot_id: u64,
    ) -> Result<(), lib::meetup::Error> {
        let redis_key_d2m = format!("discord_user:{}:meetup_user", user_id);
        let redis_connection_mutex = {
            ctx.data
                .read()
                .get::<super::bot::RedisConnectionKey>()
                .ok_or_else(|| SimpleError::new("Redis connection was not set"))?
                .clone()
        };
        let mut redis_connection = redis_connection_mutex.lock();
        // Check if there is actually a meetup id linked to this user
        let linked_meetup_id: Option<u64> = redis_connection.get(&redis_key_d2m)?;
        match linked_meetup_id {
            Some(meetup_id) => {
                let redis_key_m2d = format!("meetup_user:{}:discord_user", meetup_id);
                redis_connection.del(&[&redis_key_d2m, &redis_key_m2d])?;
                let message = if is_bot_admin_command {
                    format!("Unlinked <@{}>'s Meetup account", user_id)
                } else {
                    strings::MEETUP_UNLINK_SUCCESS(bot_id)
                };
                let _ = msg.channel_id.say(&ctx.http, message);
            }
            None => {
                let message = if is_bot_admin_command {
                    Cow::Owned(format!(
                        "There was seemingly no meetup account linked to <@{}>",
                        user_id
                    ))
                } else {
                    Cow::Borrowed(strings::MEETUP_UNLINK_NOT_LINKED)
                };
                let _ = msg.channel_id.say(&ctx.http, message);
            }
        }
        Ok(())
    }

    fn get_channel_roles(
        channel_id: u64,
        redis_connection: &mut redis::Connection,
    ) -> Result<Option<ChannelRoles>, lib::meetup::Error> {
        // Check that this message came from a bot controlled channel
        let redis_channel_role_key = format!("discord_channel:{}:discord_role", channel_id);
        let redis_channel_host_role_key =
            format!("discord_channel:{}:discord_host_role", channel_id);
        let channel_roles: redis::RedisResult<(Option<u64>, Option<u64>)> = redis::pipe()
            .get(redis_channel_role_key)
            .get(redis_channel_host_role_key)
            .query(redis_connection);
        match channel_roles {
            Ok((Some(role), Some(host_role))) => Ok(Some(ChannelRoles {
                user: role,
                host: host_role,
            })),
            Ok((None, None)) => Ok(None),
            Ok(_) => {
                return Err(SimpleError::new("Channel has only one of two roles").into());
            }
            Err(err) => {
                return Err(err.into());
            }
        }
    }

    pub fn end_adventure(
        ctx: &Context,
        msg: &Message,
        redis_client: redis::Client,
    ) -> Result<(), lib::meetup::Error> {
        let mut redis_connection = redis_client.get_connection()?;
        // Check whether this is a bot controlled channel
        let channel_roles = Self::get_channel_roles(msg.channel_id.0, &mut redis_connection)?;
        let channel_roles = match channel_roles {
            Some(roles) => roles,
            None => {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, strings::CHANNEL_NOT_BOT_CONTROLLED);
                return Ok(());
            }
        };
        // This is only for bot_admins and channel hosts
        let is_bot_admin = msg
            .author
            .has_role(
                ctx,
                lib::discord::sync::ids::GUILD_ID,
                lib::discord::sync::ids::BOT_ADMIN_ID,
            )
            .unwrap_or(false);
        let is_host = msg
            .author
            .has_role(ctx, lib::discord::sync::ids::GUILD_ID, channel_roles.host)
            .unwrap_or(false);
        if !is_bot_admin && !is_host {
            let _ = msg.channel_id.say(&ctx.http, strings::NOT_A_CHANNEL_ADMIN);
            return Ok(());
        }
        // Check if there is a channel expiration time in the future
        let redis_channel_expiration_key =
            format!("discord_channel:{}:expiration_time", msg.channel_id.0);
        let expiration_time: Option<String> =
            redis_connection.get(&redis_channel_expiration_key)?;
        let expiration_time = expiration_time
            .map(|t| chrono::DateTime::parse_from_rfc3339(&t))
            .transpose()?
            .map(|t| t.with_timezone(&chrono::Utc));
        if let Some(expiration_time) = expiration_time {
            if expiration_time > chrono::Utc::now() {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, strings::CHANNEL_NOT_YET_CLOSEABLE);
                return Ok(());
            }
        }
        // Schedule this channel for deletion
        // TODO: in 24 hours
        let new_deletion_time = chrono::Utc::now();
        let redis_channel_deletion_key =
            format!("discord_channel:{}:deletion_time", msg.channel_id.0);
        let current_deletion_time: Option<String> =
            redis_connection.get(&redis_channel_deletion_key)?;
        let current_deletion_time = current_deletion_time
            .map(|t| chrono::DateTime::parse_from_rfc3339(&t))
            .transpose()?
            .map(|t| t.with_timezone(&chrono::Utc));
        if let Some(current_deletion_time) = current_deletion_time {
            if new_deletion_time > current_deletion_time {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, strings::CHANNEL_ALREADY_MARKED_FOR_CLOSING);
                return Ok(());
            }
        }
        let _: () =
            redis_connection.set(&redis_channel_deletion_key, new_deletion_time.to_rfc3339())?;
        let _ = msg
            .channel_id
            .say(&ctx.http, strings::CHANNEL_MARKED_FOR_CLOSING);
        Ok(())
    }

    pub fn channel_add_or_remove_user(
        ctx: &Context,
        msg: &Message,
        discord_id: u64,
        add: bool,
        as_host: bool,
        redis_client: redis::Client,
    ) -> Result<(), lib::meetup::Error> {
        let mut redis_connection = redis_client.get_connection()?;
        // Check whether this is a bot controlled channel
        let channel_roles = Self::get_channel_roles(msg.channel_id.0, &mut redis_connection)?;
        let channel_roles = match channel_roles {
            Some(roles) => roles,
            None => {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, strings::CHANNEL_NOT_BOT_CONTROLLED);
                return Ok(());
            }
        };
        // This is only for bot_admins and channel hosts
        let is_bot_admin = msg
            .author
            .has_role(
                ctx,
                lib::discord::sync::ids::GUILD_ID,
                lib::discord::sync::ids::BOT_ADMIN_ID,
            )
            .unwrap_or(false);
        let is_host = msg
            .author
            .has_role(ctx, lib::discord::sync::ids::GUILD_ID, channel_roles.host)
            .unwrap_or(false);
        // Only bot admins and channel hosts can add/remove users
        if !is_bot_admin && !is_host {
            let _ = msg.channel_id.say(&ctx.http, strings::NOT_A_CHANNEL_ADMIN);
            return Ok(());
        }
        // Only bot admins can add/remove hosts
        if !is_bot_admin && as_host {
            let _ = msg.channel_id.say(&ctx.http, strings::NOT_A_BOT_ADMIN);
            return Ok(());
        }
        // Only bot admins can add users
        if !is_bot_admin && add {
            let _ = msg.channel_id.say(&ctx.http, strings::NOT_A_BOT_ADMIN);
            return Ok(());
        }
        if add {
            // Try to add the user to the channel
            match ctx.http.add_member_role(
                lib::discord::sync::ids::GUILD_ID.0,
                discord_id,
                channel_roles.user,
            ) {
                Ok(()) => {
                    let _ = msg.react(ctx, "\u{2705}");
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, format!("Welcome <@{}>!", discord_id));
                }
                Err(err) => {
                    eprintln!("Could not assign channel role: {}", err);
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, strings::CHANNEL_ROLE_ADD_ERROR);
                }
            }
            if as_host {
                match ctx.http.add_member_role(
                    lib::discord::sync::ids::GUILD_ID.0,
                    discord_id,
                    channel_roles.host,
                ) {
                    Ok(()) => {
                        let _ = msg.react(ctx, "\u{2705}");
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, strings::CHANNEL_ADDED_NEW_HOST(discord_id));
                    }
                    Err(err) => {
                        eprintln!("Could not assign channel role: {}", err);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, strings::CHANNEL_ROLE_ADD_ERROR);
                    }
                }
            }
            Ok(())
        } else {
            // Try to remove the user from the channel
            match ctx.http.remove_member_role(
                lib::discord::sync::ids::GUILD_ID.0,
                discord_id,
                channel_roles.host,
            ) {
                Ok(()) => {
                    let _ = msg.react(ctx, "\u{2705}");
                }
                Err(err) => {
                    eprintln!("Could not remove host channel role: {}", err);
                    let _ = msg
                        .channel_id
                        .say(&ctx.http, strings::CHANNEL_ROLE_REMOVE_ERROR);
                }
            }
            if !as_host {
                match ctx.http.remove_member_role(
                    lib::discord::sync::ids::GUILD_ID.0,
                    discord_id,
                    channel_roles.user,
                ) {
                    Err(err) => {
                        eprintln!("Could not remove channel role: {}", err);
                        let _ = msg
                            .channel_id
                            .say(&ctx.http, strings::CHANNEL_ROLE_REMOVE_ERROR);
                    }
                    _ => (),
                }
            }
            // Remember which users were removed manually
            if as_host {
                let redis_channel_removed_hosts_key =
                    format!("discord_channel:{}:removed_hosts", msg.channel_id.0);
                redis_connection.sadd(redis_channel_removed_hosts_key, discord_id)?;
            } else {
                let redis_channel_removed_users_key =
                    format!("discord_channel:{}:removed_users", msg.channel_id.0);
                redis_connection.sadd(redis_channel_removed_users_key, discord_id)?
            }
            Ok(())
        }
    }

    pub fn send_welcome_message(ctx: &Context, user: &User) {
        let _ = user.direct_message(ctx, |message_builder| {
            message_builder
                .content(strings::WELCOME_MESSAGE_PART1)
                .embed(|embed_builder| {
                    embed_builder
                        .colour(serenity::utils::Colour::new(0xFF1744))
                        .title(strings::WELCOME_MESSAGE_PART2_EMBED_TITLE)
                        .description(strings::WELCOME_MESSAGE_PART2_EMBED_CONTENT)
                })
        });
    }

    pub fn send_help_message(ctx: &Context, msg: &Message, bot_id: UserId) {
        let is_bot_admin = msg
            .author
            .has_role(
                ctx,
                lib::discord::sync::ids::GUILD_ID,
                lib::discord::sync::ids::BOT_ADMIN_ID,
            )
            .unwrap_or(false);
        let mut dm_result = msg
            .author
            .direct_message(ctx, |message_builder| {
                message_builder
                    .content(strings::HELP_MESSAGE_INTRO(bot_id.0))
                    .embed(|embed_builder| {
                        embed_builder
                            .colour(serenity::utils::Colour::BLUE)
                            .title(strings::HELP_MESSAGE_PLAYER_EMBED_TITLE)
                            .description(strings::HELP_MESSAGE_PLAYER_EMBED_CONTENT)
                    })
            })
            .and_then(|_| {
                msg.author.direct_message(ctx, |message_builder| {
                    message_builder.embed(|embed_builder| {
                        embed_builder
                            .colour(serenity::utils::Colour::DARK_GREEN)
                            .title(strings::HELP_MESSAGE_GM_EMBED_TITLE)
                            .description(strings::HELP_MESSAGE_GM_EMBED_CONTENT(bot_id.0))
                    })
                })
            });
        if is_bot_admin {
            dm_result = dm_result.and_then(|_| {
                msg.author.direct_message(ctx, |message_builder| {
                    message_builder.embed(|embed_builder| {
                        embed_builder
                            .colour(serenity::utils::Colour::from_rgb(255, 23, 68))
                            .title(strings::HELP_MESSAGE_ADMIN_EMBED_TITLE)
                            .description(strings::HELP_MESSAGE_ADMIN_EMBED_CONTENT(bot_id.0))
                    })
                })
            });
        }
        if let Err(err) = dm_result {
            eprintln!("Could not send help message as a DM: {}", err);
        }
        let _ = msg.react(ctx, "\u{2705}");
    }

    pub fn refresh_meetup_token(
        ctx: &Context,
        msg: &Message,
        user_id: UserId,
        mut redis_client: redis::Client,
        oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    ) {
        // Look up the Meetup ID for this user
        let mut redis_connection = match redis_client.get_connection() {
            Ok(redis_connection) => redis_connection,
            Err(err) => {
                eprintln!("Redis error when obtaining a redis connection: {}", err);
                // TODO reply
                return;
            }
        };
        let redis_discord_user_meetup_user_key = format!("discord_user:{}:meetup_user", user_id);
        let res: redis::RedisResult<Option<u64>> =
            redis_connection.get(&redis_discord_user_meetup_user_key);
        let meetup_id = match res {
            Ok(Some(meetup_id)) => meetup_id,
            Ok(None) => {
                // TODO reply
                return;
            }
            Err(err) => {
                eprintln!("Redis error when obtaining a redis connection: {}", err);
                // TODO reply
                return;
            }
        };
        let res = lib::ASYNC_RUNTIME.block_on(oauth2_consumer.refresh_oauth_tokens(
            lib::meetup::oauth2::TokenType::User(meetup_id),
            &mut redis_client,
        ));
        match res {
            Ok(_) => {
                let _ = msg.react(ctx, "\u{2705}");
            }
            Err(err) => {
                eprintln!(
                    "An error occured when trying to refresh a user's Meetup OAuth2 tokens:\n{}\n",
                    err
                );
                let _ = msg.channel_id.send_message(ctx, |message_builder| {
                    message_builder
                        .content("An error occured when trying to refresh the OAuth2 token")
                });
            }
        };
    }

    pub fn clone_event(
        ctx: &Context,
        msg: &Message,
        urlname: &str,
        meetup_event_id: &str,
        mut redis_client: redis::Client,
        async_meetup_client: Arc<AsyncMutex<Option<Arc<lib::meetup::api::AsyncClient>>>>,
        oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    ) {
        let future = async {
            // Clone the async meetup client
            let guard = async_meetup_client.lock().await;
            let client = guard.clone();
            drop(guard);
            let client = match client {
                None => {
                    return Err(lib::meetup::Error::from(simple_error::SimpleError::new(
                        "Async Meetup client not set",
                    )))
                }
                Some(client) => client,
            };
            let new_event_hook = Box::new(|new_event: lib::meetup::api::NewEvent| {
                let date_time = new_event.time.clone();
                lib::flow::ScheduleSessionFlow::new_event_hook(
                    new_event,
                    date_time,
                    meetup_event_id,
                )
            });
            let new_event = lib::meetup::util::clone_event(
                urlname,
                meetup_event_id,
                client.as_ref(),
                Some(new_event_hook),
            )
            .await?;
            // Try to transfer the RSVPs to the new event
            if let Err(_) = lib::meetup::util::clone_rsvps(
                urlname,
                meetup_event_id,
                &new_event.id,
                &mut redis_client,
                client.as_ref(),
                oauth2_consumer.as_ref(),
            )
            .await
            {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, "Could not transfer all RSVPs to the new event");
            }
            Ok(new_event)
        };
        match lib::ASYNC_RUNTIME.block_on(future) {
            Ok(new_event) => {
                let _ = msg.react(ctx, "\u{2705}");
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!("Created new Meetup event: {}", new_event.link),
                );
            }
            Err(err) => {
                let _ = msg.channel_id.say(&ctx.http, "Something went wrong");
                eprintln!(
                    "Could not clone Meetup event {}. Error:\n{:#?}",
                    meetup_event_id, err
                );
            }
        }
    }

    pub fn rsvp_user_to_event(
        ctx: &Context,
        msg: &Message,
        user_id: UserId,
        urlname: &str,
        meetup_event_id: &str,
        mut redis_client: redis::Client,
        oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    ) {
        // Look up the Meetup ID for this user
        let mut redis_connection = match redis_client.get_connection() {
            Ok(redis_connection) => redis_connection,
            Err(err) => {
                eprintln!("Redis error when obtaining a redis connection: {}", err);
                // TODO reply
                return;
            }
        };
        let redis_discord_user_meetup_user_key = format!("discord_user:{}:meetup_user", user_id);
        let res: redis::RedisResult<Option<u64>> =
            redis_connection.get(&redis_discord_user_meetup_user_key);
        let meetup_id = match res {
            Ok(Some(meetup_id)) => meetup_id,
            Ok(None) => {
                // TODO reply
                return;
            }
            Err(err) => {
                eprintln!("Redis error when looking up the user's meetup ID: {}", err);
                // TODO reply
                return;
            }
        };
        // Try to RSVP the user
        match lib::ASYNC_RUNTIME.block_on(lib::meetup::util::rsvp_user_to_event(
            meetup_id,
            urlname,
            meetup_event_id,
            &mut redis_client,
            oauth2_consumer.as_ref(),
        )) {
            Ok(rsvp) => {
                let _ = msg.react(ctx, "\u{2705}");
                println!(
                    "RSVP'd Meetup user {} to Meetup event {}. RSVP:\n{:#?}",
                    meetup_id, meetup_event_id, rsvp
                );
            }
            Err(err) => {
                let _ = msg.channel_id.say(&ctx.http, "Something went wrong");
                eprintln!(
                    "Could not RSVP Meetup user {} to Meetup event {}. Error:\n{:#?}",
                    meetup_id, meetup_event_id, err
                );
            }
        }
    }

    pub fn whois_by_discord_id(
        ctx: &Context,
        msg: &Message,
        user_id: UserId,
        mut redis_client: redis::Client,
    ) {
        let redis_discord_meetup_key = format!("discord_user:{}:meetup_user", user_id.0);
        let res: redis::RedisResult<Option<String>> = redis_client.get(&redis_discord_meetup_key);
        match res {
            Ok(Some(meetup_id)) => {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!(
                        "<@{}> is linked to https://www.meetup.com/members/{}/",
                        user_id.0, meetup_id
                    ),
                );
            }
            Ok(None) => {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!(
                        "<@{}> does not seem to be linked to a Meetup account",
                        user_id.0
                    ),
                );
            }
            Err(err) => {
                eprintln!(
                    "Error when trying to look up a Meetup user in Redis:\n{:#?}",
                    err
                );
                let _ = msg.channel_id.say(&ctx.http, "Something went wrong");
            }
        }
    }

    pub fn whois_by_discord_username_tag(
        ctx: &Context,
        msg: &Message,
        username_tag: &str,
        redis_client: redis::Client,
    ) {
        if let Some(guild) = ctx
            .cache
            .read()
            .guilds
            .get(&lib::discord::sync::ids::GUILD_ID)
            .cloned()
        {
            let discord_id = guild
                .read()
                .member_named(username_tag)
                .map(|m| m.user.read().id);
            if let Some(discord_id) = discord_id {
                // Look up by Discord ID
                Self::whois_by_discord_id(ctx, msg, discord_id, redis_client);
            } else {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, format!("{} is not a Discord user", username_tag));
            }
        } else {
            let _ = msg
                .channel_id
                .say(&ctx.http, "Something went wrong (guild not found)");
        }
    }

    pub fn whois_by_meetup_id(
        ctx: &Context,
        msg: &Message,
        meetup_id: u64,
        mut redis_client: redis::Client,
    ) {
        let redis_meetup_discord_key = format!("meetup_user:{}:discord_user", meetup_id);
        let res: redis::RedisResult<Option<String>> = redis_client.get(&redis_meetup_discord_key);
        match res {
            Ok(Some(discord_id)) => {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!(
                        "https://www.meetup.com/members/{}/ is linked to <@{}>",
                        meetup_id, discord_id
                    ),
                );
            }
            Ok(None) => {
                let _ = msg.channel_id.say(
                    &ctx.http,
                    format!(
                        "https://www.meetup.com/members/{}/ does not seem to be linked to a \
                         Discord user",
                        meetup_id
                    ),
                );
            }
            Err(err) => {
                eprintln!(
                    "Error when trying to look up a Discord user in Redis:\n{:#?}",
                    err
                );
                let _ = msg.channel_id.say(&ctx.http, "Something went wrong");
            }
        }
    }

    pub fn schedule_session(
        ctx: &Context,
        msg: &Message,
        redis_client: &mut redis::Client,
    ) -> Result<(), lib::meetup::Error> {
        let mut redis_connection = redis_client.get_connection()?;
        // Check whether this is a bot controlled channel
        let channel_roles = Self::get_channel_roles(msg.channel_id.0, &mut redis_connection)?;
        let channel_roles = match channel_roles {
            Some(roles) => roles,
            None => {
                let _ = msg
                    .channel_id
                    .say(&ctx.http, strings::CHANNEL_NOT_BOT_CONTROLLED);
                return Ok(());
            }
        };
        // This is only for channel hosts
        let is_host = msg
            .author
            .has_role(ctx, lib::discord::sync::ids::GUILD_ID, channel_roles.host)
            .unwrap_or(false);
        if !is_host {
            let _ = msg.channel_id.say(&ctx.http, strings::NOT_A_CHANNEL_ADMIN);
            return Ok(());
        }
        // Find the series belonging to the channel
        let redis_channel_series_key = format!("discord_channel:{}:event_series", msg.channel_id.0);
        let event_series: String = redis_client.get(&redis_channel_series_key)?;
        // Create a new Flow
        let flow = lib::flow::ScheduleSessionFlow::new(redis_client, event_series)?;
        let link = format!("{}/schedule_session/{}", lib::urls::BASE_URL, flow.id);
        let _ = msg.author.direct_message(ctx, |message_builder| {
            message_builder.content(format!(
                "Use the following link to schedule your next session:\n{}",
                link
            ))
        });
        let _ = msg.react(ctx, "\u{2705}");
        Ok(())
    }
}

struct ChannelRoles {
    user: u64,
    host: u64,
}