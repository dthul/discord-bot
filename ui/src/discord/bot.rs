use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use futures_util::lock::Mutex as AsyncMutex;
use lib::strings;
use serenity::{
    all::{ApplicationId, GuildMemberUpdateEvent},
    async_trait,
    builder::CreateMessage,
    model::{
        application::Interaction,
        channel::Message,
        gateway::{GatewayIntents, Ready},
        guild::Member,
        id::{GuildId, UserId},
        user::User,
    },
    prelude::*,
};
use tracing::{error, info};

use super::commands::{CommandContext, PreparedCommands};

#[tracing::instrument(skip(
    redis_client,
    pool,
    async_meetup_client,
    oauth2_consumer,
    stripe_client,
    shutdown_signal
))]
pub async fn create_discord_client(
    discord_token: &str,
    application_id: ApplicationId,
    redis_client: redis::Client,
    pool: sqlx::PgPool,
    async_meetup_client: Arc<AsyncMutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>,
    oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    stripe_client: Arc<stripe::Client>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<Client, lib::meetup::Error> {
    // Create a new instance of the Client, logging in as a bot. This will
    // automatically prepend your bot token with "Bot ", which is a requirement
    // by Discord for bot users.
    let client = Client::builder(
        &discord_token,
        GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MEMBERS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::DIRECT_MESSAGE_REACTIONS
            | GatewayIntents::GUILD_PRESENCES
            | GatewayIntents::GUILD_VOICE_STATES,
    )
    .application_id(application_id)
    .event_handler(Handler)
    .await?;

    // We will fetch the bot's id.
    let (bot_id, bot_name) = client
        .http
        .get_current_user()
        .await
        .map(|info| (info.id, info.name.clone()))?;
    info!("Bot ID: {}", bot_id);
    info!("Bot name: {}", bot_name);

    // Prepare the commands
    let prepared_commands = Arc::new(super::commands::prepare_commands(bot_id, &bot_name)?);

    // Store the data to be shared by command invocations
    {
        let mut data = client.data.write().await;
        data.insert::<BotIdKey>(bot_id);
        data.insert::<BotNameKey>(bot_name);
        data.insert::<AsyncMeetupClientKey>(async_meetup_client);
        data.insert::<RedisClientKey>(redis_client);
        data.insert::<PoolKey>(pool);
        data.insert::<OAuth2ConsumerKey>(oauth2_consumer);
        data.insert::<StripeClientKey>(stripe_client);
        data.insert::<ShutdownSignalKey>(shutdown_signal);
        data.insert::<PreparedCommandsKey>(prepared_commands);
    }

    Ok(client)
}

pub struct BotIdKey;
impl TypeMapKey for BotIdKey {
    type Value = UserId;
}

pub struct BotNameKey;
impl TypeMapKey for BotNameKey {
    type Value = String;
}

pub struct AsyncMeetupClientKey;
impl TypeMapKey for AsyncMeetupClientKey {
    type Value = Arc<AsyncMutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>;
}

pub struct RedisClientKey;
impl TypeMapKey for RedisClientKey {
    type Value = redis::Client;
}

pub struct OAuth2ConsumerKey;
impl TypeMapKey for OAuth2ConsumerKey {
    type Value = Arc<lib::meetup::oauth2::OAuth2Consumer>;
}

pub struct StripeClientKey;
impl TypeMapKey for StripeClientKey {
    type Value = Arc<stripe::Client>;
}

pub struct ShutdownSignalKey;
impl TypeMapKey for ShutdownSignalKey {
    type Value = Arc<AtomicBool>;
}

pub(crate) struct PreparedCommandsKey;
impl TypeMapKey for PreparedCommandsKey {
    type Value = Arc<PreparedCommands>;
}

pub(crate) struct PoolKey;
impl TypeMapKey for PoolKey {
    type Value = sqlx::PgPool;
}

pub struct Handler;

impl Handler {
    #[tracing::instrument(skip_all, fields(message.id = %cmdctx.msg.id, message.channel = %cmdctx.msg.channel_id, message.author.id = %cmdctx.msg.author.id, message.author.name = cmdctx.msg.author.name, message.content = cmdctx.msg.content))]
    async fn handle_message(
        cmdctx: &mut CommandContext,
        commands: Arc<PreparedCommands>,
    ) -> Result<(), lib::meetup::Error> {
        // Figure out which command matches
        let matches: Vec<_> = commands
            .regex_set
            .matches(&cmdctx.msg.content)
            .into_iter()
            .collect();
        let bot_id = cmdctx.bot_id().await?;
        let i = match matches.as_slice() {
            [] => {
                // unknown command
                error!("Unrecognized command: {}", &cmdctx.msg.content);
                cmdctx
                    .msg
                    .channel_id
                    .say(&cmdctx.ctx, strings::INVALID_COMMAND(bot_id))
                    .await
                    .ok();
                return Ok(());
            }
            [i] => *i, // unique command found
            l @ _ => {
                // multiple commands found
                error!(
                    "Ambiguous command: {}. Matching regexes: {:#?}",
                    &cmdctx.msg.content, l
                );
                let _ = cmdctx.msg.channel_id.say(
                    &cmdctx.ctx,
                    "I can't figure out what to do. This is a bug. Could you please let a bot \
                     admin know about this?",
                );
                return Ok(());
            }
        };
        // We clone the message's content here, such that we don't keep a
        // reference to the message object
        let message_content = cmdctx.msg.content.clone();
        let captures = match commands.regexes[i].captures(&message_content) {
            Some(captures) => captures,
            None => {
                // This should not happen
                error!("Unmatcheable command: {}", &message_content);
                let _ = cmdctx.msg.channel_id.say(
                    &cmdctx.ctx,
                    "I can't parse your command. This is a bug. Could you please let a bot admin \
                     know about this?",
                );
                return Ok(());
            }
        };
        // Check whether the user has the required permissions
        let command = commands.commands[i];
        match command.level {
            super::commands::CommandLevel::Everybody => (),
            super::commands::CommandLevel::AdminOnly => {
                if !cmdctx.is_admin().await? {
                    cmdctx
                        .msg
                        .channel_id
                        .say(&cmdctx.ctx, strings::NOT_A_BOT_ADMIN)
                        .await
                        .ok();
                    return Ok(());
                }
            }
            super::commands::CommandLevel::HostAndAdminOnly => {
                let is_admin = cmdctx.is_admin().await?;
                let is_host = cmdctx.is_host().await?;
                if !is_admin && !is_host {
                    cmdctx
                        .msg
                        .channel_id
                        .say(&cmdctx.ctx, strings::NOT_A_CHANNEL_ADMIN)
                        .await
                        .ok();
                    return Ok(());
                }
            }
        }
        // Call the command
        (command.fun)(cmdctx, captures).await?;
        Ok(())
    }
}

#[async_trait]
impl EventHandler for Handler {
    // Set a handler for the `message` event - so that whenever a new message
    // is received - the closure (or function) passed will be called.
    //
    // Event handlers are dispatched through a threadpool, and so multiple
    // events can be dispatched simultaneously.
    async fn message(&self, ctx: Context, msg: Message) {
        let (bot_id, shutdown_signal) = {
            let data = ctx.data.read().await;
            let bot_id = data.get::<BotIdKey>().expect("Bot ID was not set").clone();
            let shutdown_signal = data
                .get::<ShutdownSignalKey>()
                .expect("Shutdown signal was not set")
                .load(Ordering::Acquire);
            (bot_id, shutdown_signal)
        };
        // Ignore all messages written by the bot itself
        if msg.author.id == bot_id {
            return;
        }
        // Ignore all messages that might have come from another guild
        // (shouldn't happen, but who knows)
        if let Some(guild_id) = msg.guild_id {
            if guild_id != lib::discord::sync::ids::GUILD_ID {
                return;
            }
        }
        // Get the prepared commands
        let commands = ctx
            .data
            .read()
            .await
            .get::<PreparedCommandsKey>()
            .cloned()
            .expect("Prepared commands have not been set");
        // Wrap Serenity's context and message objects into a CommandContext
        // for access to convenience functions.
        let mut cmdctx = CommandContext::new(ctx, msg);
        // Is this a direct message to the bot?
        let is_dm = match cmdctx.is_dm().await {
            Ok(is_dm) => is_dm,
            Err(err) => {
                error!(
                    "Could not figure out whether this message is a DM:\n{:#?}",
                    err
                );
                cmdctx
                    .msg
                    .channel_id
                    .say(&cmdctx.ctx, lib::strings::UNSPECIFIED_ERROR)
                    .await
                    .ok();
                return;
            }
        };
        // Does the message start with a mention of the bot?
        let is_mention = commands.bot_mention.is_match(&cmdctx.msg.content);
        if !is_dm {
            // Forward this message to the spam hook
            super::spam::message_hook(&mut cmdctx).await.ok();
        }
        // If the message is not a direct message and does not start with a
        // mention of the bot, ignore it
        if !is_dm && !is_mention {
            return;
        }
        if shutdown_signal {
            let _ = cmdctx.msg.channel_id.say(
                &cmdctx.ctx,
                "Sorry, I can not help you right now. I am about to shut down!",
            );
            return;
        }
        // Poor man's try block
        let res: Result<(), lib::meetup::Error> = Self::handle_message(&mut cmdctx, commands).await;
        if let Err(err) = res {
            error!("Error in message handler:\n{:#?}", err);
            let _ = cmdctx
                .msg
                .channel_id
                .say(&cmdctx.ctx, lib::strings::UNSPECIFIED_ERROR);
        }
    }

    // Set a handler to be called on the `ready` event. This is called when a
    // shard is booted, and a READY payload is sent by Discord. This payload
    // contains data like the current user's guild Ids, current user data,
    // private channels, and more.
    //
    // In this case, just print what the current user's username is.
    async fn ready(&self, _: Context, ready: Ready) {
        info!("{} is connected!", ready.user.name);
    }

    #[tracing::instrument(skip_all, fields(member.nick = new_member.nick, member.user.id = %new_member.user.id, member.user.name = %new_member.user.name))]
    async fn guild_member_addition(&self, ctx: Context, new_member: Member) {
        if new_member.guild_id != lib::discord::sync::ids::GUILD_ID {
            return;
        }
        Self::send_welcome_message(&ctx, &new_member.user).await;
        let nick = new_member
            .nick
            .as_deref()
            .unwrap_or(new_member.user.name.as_str());
        Self::update_member_nick(&ctx, new_member.user.id, nick)
            .await
            .ok();
    }

    async fn guild_member_update(
        &self,
        ctx: Context,
        _old_if_available: Option<Member>,
        _new: Option<Member>,
        event: GuildMemberUpdateEvent,
    ) {
        let nick = event.nick.as_deref().unwrap_or(event.user.name.as_str());
        Self::update_member_nick(&ctx, event.user.id, nick)
            .await
            .ok();
    }

    async fn cache_ready(&self, ctx: Context, guilds: Vec<GuildId>) {
        let guild_id = match guilds.as_slice() {
            [guild] => guild,
            _ => {
                error!("cache_ready event received not exactly one guild");
                return;
            }
        };
        let members = match ctx.cache.guild(guild_id).map(|guild| guild.members.clone()) {
            Some(members) => members,
            None => return,
        };
        info!("Updating {} cached member nicks", members.len());
        for (user_id, member) in members {
            let nick = member.nick.as_deref().unwrap_or(member.user.name.as_str());
            Self::update_member_nick(&ctx, user_id, nick).await.ok();
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        // let (bot_id, shutdown_signal) = {
        //     let data = ctx.data.read().await;
        //     let bot_id = data.get::<BotIdKey>().expect("Bot ID was not set").clone();
        //     let shutdown_signal = data
        //         .get::<ShutdownSignalKey>()
        //         .expect("Shutdown signal was not set")
        //         .load(Ordering::Acquire);
        //     (bot_id, shutdown_signal)
        // };
        // In contrast to the message handler we don't need to check that this
        // is indeed a command.
        let interaction = match interaction {
            Interaction::Command(inner) => inner,
            _ => return,
        };

        // Ignore all messages that might have come from another guild
        // (shouldn't happen, but who knows)
        if interaction.guild_id != Some(lib::discord::sync::ids::GUILD_ID) {
            return;
        }
        match interaction.data.name.as_str() {
            "link-meetup" => {
                interaction
                    .channel_id
                    .say(&ctx, "Yessir! Linking Meetup")
                    .await
                    .ok();
            }
            "unlink-meetup" => {
                interaction
                    .channel_id
                    .say(&ctx, "Un-linking Meetup")
                    .await
                    .ok();
            }
            _ => {
                interaction
                    .channel_id
                    .say(&ctx, "Unknown command")
                    .await
                    .ok();
            }
        };
    }
}

impl Handler {
    #[tracing::instrument(skip(ctx), fields(user.id = %user.id, user.name = user.name))]
    async fn send_welcome_message(ctx: &Context, user: &User) {
        user.direct_message(ctx, CreateMessage::new().content(strings::WELCOME_MESSAGE))
            .await
            .ok();
    }

    #[tracing::instrument(skip(ctx))]
    async fn update_member_nick(
        ctx: &Context,
        discord_id: UserId,
        nick: &str,
    ) -> Result<(), lib::meetup::Error> {
        let pool = {
            let data = ctx.data.read().await;
            data.get::<super::bot::PoolKey>()
                .cloned()
                .ok_or_else(|| simple_error::SimpleError::new("Postgres pool was not set"))?
        };
        let mut tx = pool.begin().await?;
        let member_id = lib::db::get_or_create_member_for_discord_id(&mut tx, discord_id).await?;
        sqlx::query!(
            r#"UPDATE member SET discord_nick = $2 WHERE id = $1 AND discord_nick IS DISTINCT FROM $2"#,
            member_id.0,
            nick
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
