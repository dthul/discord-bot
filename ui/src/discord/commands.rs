use std::{future::Future, pin::Pin, sync::Arc};

use futures_util::lock::Mutex as AsyncMutex;
use once_cell::sync::OnceCell;
use regex::{Regex, RegexSet};
use serenity::{
    model::{
        channel::{Channel, Message},
        id::UserId,
    },
    prelude::*,
};

mod add_user;
// mod clone_event;
mod count_inactive;
mod end_adventure;
#[cfg(feature = "bottest")]
mod end_all;
mod help;
mod link_meetup;
mod list_players;
mod list_subscriptions;
mod login;
mod manage_channel;
// mod mention_channel;
mod numcached;
// mod refresh_meetup_token;
mod remind_expiration;
mod schedule_session;
mod snooze;
mod stop;
mod sync_discord;
mod sync_meetup;
mod sync_subscriptions;
mod topic;
// mod test;
mod whois;

static ALL_COMMANDS: &[&Command] = &[
    &stop::STOP_COMMAND,
    &link_meetup::LINK_MEETUP_COMMAND,
    &link_meetup::UNLINK_MEETUP_COMMAND,
    &link_meetup::LINK_MEETUP_BOT_ADMIN_COMMAND,
    &link_meetup::UNLINK_MEETUP_BOT_ADMIN_COMMAND,
    &topic::SET_VOICE_TOPIC_COMMAND,
    &sync_meetup::SYNC_MEETUP_COMMAND,
    &sync_discord::SYNC_DISCORD_COMMAND,
    &remind_expiration::REMIND_EXPIRATION_COMMAND,
    &add_user::ADD_USER_COMMAND,
    &add_user::ADD_HOST_COMMAND,
    &add_user::REMOVE_USER_COMMAND,
    &add_user::REMOVE_HOST_COMMAND,
    &end_adventure::END_ADVENTURE_COMMAND,
    &help::HELP_COMMAND,
    // &refresh_meetup_token::REFRESH_MEETUP_TOKEN_COMMAND,
    &schedule_session::SCHEDULE_SESSION_COMMAND,
    &whois::WHOIS_COMMAND,
    &list_players::LIST_PLAYERS_COMMAND,
    #[cfg(feature = "bottest")]
    &end_all::END_ALL_COMMAND,
    &list_subscriptions::LIST_SUBSCRIPTIONS_COMMAND,
    &sync_subscriptions::SYNC_SUBSCRIPTIONS_COMMAND,
    &numcached::NUMCACHED_COMMAND,
    &manage_channel::MANAGE_CHANNEL_COMMAND,
    // &mention_channel::MENTION_CHANNEL_COMMAND,
    &snooze::SNOOZE_COMMAND,
    &count_inactive::COUNT_INACTIVE_COMMAND,
    &count_inactive::COUNT_MEMBERS_COMMAND,
    // &clone_event::CLONE_EVENT_COMMAND,
    // &test::TEST_COMMAND,
    &login::LOGIN_COMMAND,
];

const MENTION_PATTERN: &'static str = r"(?:<@!?(?P<mention_id>[0-9]+)>)";
const USERNAME_TAG_PATTERN: &'static str = r"(?P<discord_username_tag>[^@#:]{2,32}#[0-9]+)";
const USERNAME_PATTERN: &'static str = r"(?P<discord_username>[A-Za-z0-9_\.]{2,32})";
const MEETUP_ID_PATTERN: &'static str = r"(?P<meetup_user_id>[0-9]+)";

pub(crate) enum CommandLevel {
    Everybody,
    HostAndAdminOnly,
    AdminOnly,
}

pub struct RegexParts<'a> {
    mention_pattern: &'a str,
    username_tag_pattern: &'a str,
    username_pattern: &'a str,
    meetup_id_pattern: &'a str,
}

type CommandResult<'a> = Pin<Box<dyn Future<Output = Result<(), lib::meetup::Error>> + Send + 'a>>;

pub(crate) struct Command {
    pub regex: fn(&RegexParts<'_>) -> String,
    pub level: CommandLevel,
    pub fun: &'static (dyn for<'a> Fn(&'a mut CommandContext, regex::Captures<'a>) -> CommandResult<'a>
                  + Sync
                  + 'static),
    pub help: &'static [HelpEntry],
}

pub(crate) struct HelpEntry {
    pub command: &'static str,
    pub explanation: &'static str,
}

pub struct CommandContext {
    pub ctx: Context,
    pub msg: Message,
    // pub captures: regex::Captures<'a>,
    redis_client: OnceCell<redis::Client>,
    async_redis_connection: OnceCell<redis::aio::MultiplexedConnection>,
    meetup_client: OnceCell<Arc<AsyncMutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>>,
    oauth2_consumer: OnceCell<Arc<lib::meetup::oauth2::OAuth2Consumer>>,
    stripe_client: OnceCell<Arc<stripe::Client>>,
    bot_id: OnceCell<UserId>,
    channel: OnceCell<Channel>,
    pool: OnceCell<sqlx::PgPool>,
}

impl CommandContext {
    pub fn new(ctx: Context, msg: Message) -> Self {
        CommandContext {
            ctx,
            msg,
            redis_client: OnceCell::new(),
            async_redis_connection: OnceCell::new(),
            meetup_client: OnceCell::new(),
            oauth2_consumer: OnceCell::new(),
            stripe_client: OnceCell::new(),
            bot_id: OnceCell::new(),
            channel: OnceCell::new(),
            pool: OnceCell::new(),
        }
    }

    pub async fn redis_client(&self) -> Result<redis::Client, lib::meetup::Error> {
        if let Some(client) = self.redis_client.get() {
            Ok(client.clone())
        } else {
            let data = self.ctx.data.read().await;
            let client = data
                .get::<super::bot::RedisClientKey>()
                .cloned()
                .ok_or_else(|| simple_error::SimpleError::new("Redis client was not set"))?;
            Ok(self.redis_client.get_or_init(move || client).clone())
        }
    }

    pub async fn async_redis_connection<'b>(
        &'b mut self,
    ) -> Result<&'b mut redis::aio::MultiplexedConnection, lib::meetup::Error> {
        if self.async_redis_connection.get().is_some() {
            Ok(self
                .async_redis_connection
                .get_mut()
                .expect("Async redis connection not set. This is a bug."))
        } else {
            let redis_client = self.redis_client().await?;
            let async_redis_connection = redis_client.get_multiplexed_async_connection().await?;
            self.async_redis_connection.set(async_redis_connection).ok();
            Ok(self
                .async_redis_connection
                .get_mut()
                .expect("Async redis connection not set. This is a bug."))
        }
    }

    pub async fn pool(&self) -> Result<sqlx::PgPool, lib::meetup::Error> {
        if let Some(pool) = self.pool.get() {
            Ok(pool.clone())
        } else {
            let data = self.ctx.data.read().await;
            let pool = data
                .get::<super::bot::PoolKey>()
                .cloned()
                .ok_or_else(|| simple_error::SimpleError::new("Postgres pool was not set"))?;
            Ok(self.pool.get_or_init(move || pool).clone())
        }
    }

    pub async fn meetup_client(
        &self,
    ) -> Result<Arc<AsyncMutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>, lib::meetup::Error>
    {
        if let Some(client) = self.meetup_client.get() {
            Ok(Arc::clone(client))
        } else {
            let client = self
                .ctx
                .data
                .read()
                .await
                .get::<super::bot::AsyncMeetupClientKey>()
                .cloned()
                .ok_or_else(|| simple_error::SimpleError::new("Meetup client was not set"))?;
            self.meetup_client.set(client).ok();
            Ok(self
                .meetup_client
                .get()
                .map(Arc::clone)
                .expect("Meetup client not set. This is a bug."))
        }
    }

    pub async fn oauth2_consumer(
        &self,
    ) -> Result<Arc<lib::meetup::oauth2::OAuth2Consumer>, lib::meetup::Error> {
        if let Some(consumer) = self.oauth2_consumer.get() {
            Ok(Arc::clone(consumer))
        } else {
            let consumer = self
                .ctx
                .data
                .read()
                .await
                .get::<super::bot::OAuth2ConsumerKey>()
                .cloned()
                .ok_or_else(|| simple_error::SimpleError::new("OAuth2 consumer was not set"))?;
            self.oauth2_consumer.set(consumer).ok();
            Ok(self
                .oauth2_consumer
                .get()
                .map(Arc::clone)
                .expect("OAuth2 consumer not set. This is a bug."))
        }
    }

    pub async fn stripe_client(&self) -> Result<Arc<stripe::Client>, lib::meetup::Error> {
        if let Some(client) = self.stripe_client.get() {
            Ok(Arc::clone(client))
        } else {
            let client = self
                .ctx
                .data
                .read()
                .await
                .get::<super::bot::StripeClientKey>()
                .cloned()
                .ok_or_else(|| simple_error::SimpleError::new("Stripe client was not set"))?;
            self.stripe_client.set(client).ok();
            Ok(self
                .stripe_client
                .get()
                .map(Arc::clone)
                .expect("Stripe client not set. This is a bug."))
        }
    }

    pub async fn bot_id(&self) -> Result<UserId, lib::meetup::Error> {
        if let Some(&bot_id) = self.bot_id.get() {
            Ok(bot_id)
        } else {
            let bot_id = self
                .ctx
                .data
                .read()
                .await
                .get::<super::bot::BotIdKey>()
                .copied()
                .ok_or_else(|| simple_error::SimpleError::new("Bot ID was not set"))?;
            self.bot_id.set(bot_id).ok();
            Ok(bot_id)
        }
    }

    pub async fn channel(&self) -> Result<Channel, lib::meetup::Error> {
        if let Some(channel) = self.channel.get() {
            Ok(channel.clone())
        } else {
            let channel = self.msg.channel_id.to_channel(&self.ctx).await?;
            self.channel.set(channel.clone()).ok();
            Ok(channel)
        }
    }

    pub async fn is_dm(&self) -> Result<bool, lib::meetup::Error> {
        Ok(match self.channel().await? {
            Channel::Private(_) => true,
            _ => false,
        })
    }

    pub async fn is_admin(&self) -> Result<bool, lib::meetup::Error> {
        Ok(self
            .msg
            .author
            .has_role(
                &self.ctx,
                lib::discord::sync::ids::GUILD_ID,
                lib::discord::sync::ids::BOT_ADMIN_ID,
            )
            .await?)
    }

    pub async fn is_host(&mut self) -> Result<bool, lib::meetup::Error> {
        let ctx = (&self.ctx).into();
        let channel_id = self.msg.channel_id;
        let user_id = self.msg.author.id;
        let pool = self.pool().await?;
        lib::discord::is_host(&ctx, channel_id, user_id, &pool).await
    }

    pub async fn is_game_channel(
        &mut self,
        tx: Option<&mut sqlx::Transaction<'_, sqlx::Postgres>>,
    ) -> Result<bool, lib::meetup::Error> {
        if let Some(tx) = tx {
            lib::is_game_channel(self.msg.channel_id, tx).await
        } else {
            let mut connection = self.pool().await?.acquire().await?;
            lib::is_game_channel(self.msg.channel_id, &mut connection).await
        }
    }

    pub async fn is_managed_channel(&mut self) -> Result<bool, lib::meetup::Error> {
        let channel_id = self.msg.channel_id;
        let pool = self.pool().await?;
        Ok(sqlx::query_scalar!(
            r#"SELECT COUNT(*) > 0 AS "is_managed_channel!" FROM managed_channel WHERE discord_id = $1"#,
            channel_id.get() as i64
        )
        .fetch_one(&pool)
        .await?)
    }
}

pub(crate) struct PreparedCommands {
    pub regex_set: RegexSet,
    pub regexes: Vec<Regex>,
    pub commands: Vec<&'static Command>,
    pub bot_mention: Regex,
}

pub(crate) fn prepare_commands(
    bot_id: UserId,
    bot_name: &str,
) -> Result<PreparedCommands, lib::meetup::Error> {
    let regex_parts = RegexParts {
        mention_pattern: MENTION_PATTERN,
        username_tag_pattern: USERNAME_TAG_PATTERN,
        username_pattern: USERNAME_PATTERN,
        meetup_id_pattern: MEETUP_ID_PATTERN,
    };
    let bot_mention = format!(
        r"(?:<@!?{bot_id}>|(@|#)(?i){bot_name})",
        bot_id = bot_id.get(),
        bot_name = regex::escape(bot_name)
    );
    let mut commands = vec![];
    let mut regexes = vec![];
    for &command in ALL_COMMANDS {
        let command_partial_regex = (command.regex)(&regex_parts);
        let command_dm_regex = format!(r"^\s*(?i){command}\s*$", command = command_partial_regex);
        let command_mention_regex = format!(
            r"^\s*{bot_mention}\s+(?i){command}\s*$",
            bot_mention = bot_mention,
            command = command_partial_regex
        );
        match (
            Regex::new(&command_dm_regex),
            Regex::new(&command_mention_regex),
        ) {
            (Ok(dm_regex), Ok(mention_regex)) => {
                regexes.push(dm_regex);
                commands.push(command);
                regexes.push(mention_regex);
                commands.push(command);
            }
            (res1, res2) => {
                let err = res1.err().unwrap_or_else(|| res2.unwrap_err());
                eprintln!(
                    "Could not compile command regex \"{}\":\n{:#?}",
                    command_partial_regex, err
                )
            }
        }
    }
    let regex_set = regex::RegexSet::new(regexes.iter().map(Regex::as_str))?;
    Ok(PreparedCommands {
        regex_set,
        regexes,
        commands,
        bot_mention: Regex::new(&format!(r"^\s*{}", bot_mention))?,
    })
}
