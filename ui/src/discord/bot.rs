use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, OnceLock,
};

use futures_util::lock::Mutex as AsyncMutex;
use lib::strings;
use serenity::{
    all::GuildMemberUpdateEvent,
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

use super::commands::{CommandContext, PreparedCommands};
use super::spam::{GameChannelsList, SpamList};

pub struct UserData {
    pub bot_id: UserId,
    pub bot_name: String,
    pub async_meetup_client: Arc<AsyncMutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>,
    pub redis_client: redis::Client,
    pub pool: sqlx::PgPool,
    pub oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    pub stripe_client: Arc<stripe::Client>,
    pub shutdown_signal: Arc<AtomicBool>,
    pub(crate) prepared_commands: Arc<PreparedCommands>,
    pub spam_list: OnceLock<SpamList>,
    pub(super) games_list: RwLock<Arc<GameChannelsList>>,
}

pub async fn create_discord_client(
    discord_token: &str,
    redis_client: redis::Client,
    pool: sqlx::PgPool,
    async_meetup_client: Arc<AsyncMutex<Option<Arc<lib::meetup::newapi::AsyncClient>>>>,
    oauth2_consumer: Arc<lib::meetup::oauth2::OAuth2Consumer>,
    stripe_client: Arc<stripe::Client>,
    shutdown_signal: Arc<AtomicBool>,
) -> Result<Client, lib::meetup::Error> {
    let http = serenity::http::HttpBuilder::new(discord_token).build();

    // We will fetch the bot's id.
    let (bot_id, bot_name) = http
        .get_current_user()
        .await
        .map(|info| (info.id, info.name.clone()))?;
    println!("Bot ID: {}", bot_id);
    println!("Bot name: {}", bot_name);

    // Prepare the commands
    let prepared_commands = Arc::new(super::commands::prepare_commands(bot_id, &bot_name)?);

    // Create a new instance of the Client, logging in as a bot. This will
    // automatically prepend your bot token with "Bot ", which is a requirement
    // by Discord for bot users.
    let client = Client::builder(
        discord_token,
        GatewayIntents::GUILDS
            | GatewayIntents::GUILD_MEMBERS
            | GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::GUILD_MESSAGE_REACTIONS
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::DIRECT_MESSAGE_REACTIONS
            | GatewayIntents::GUILD_PRESENCES
            | GatewayIntents::GUILD_VOICE_STATES,
    )
    .event_handler(Handler)
    .data(Arc::new(UserData {
        bot_id: bot_id,
        bot_name: bot_name.into(),
        async_meetup_client: async_meetup_client,
        redis_client: redis_client,
        pool: pool,
        oauth2_consumer: oauth2_consumer,
        stripe_client: stripe_client,
        shutdown_signal: shutdown_signal,
        prepared_commands: prepared_commands,
        spam_list: Default::default(),
        games_list: Default::default(),
    }))
    .await?;

    Ok(client)
}

pub struct Handler;

impl Handler {
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
        let i = match matches.as_slice() {
            [] => {
                // unknown command
                eprintln!("Unrecognized command: {}", &cmdctx.msg.content);
                cmdctx
                    .msg
                    .channel_id
                    .say(&cmdctx.ctx.http, strings::INVALID_COMMAND(cmdctx.bot_id()))
                    .await
                    .ok();
                return Ok(());
            }
            [i] => *i, // unique command found
            l @ _ => {
                // multiple commands found
                eprintln!(
                    "Ambiguous command: {}. Matching regexes: {:#?}",
                    &cmdctx.msg.content, l
                );
                let _ = cmdctx.msg.channel_id.say(
                    &cmdctx.ctx.http,
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
                eprintln!("Unmatcheable command: {}", &message_content);
                let _ = cmdctx.msg.channel_id.say(
                    &cmdctx.ctx.http,
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
                        .say(&cmdctx.ctx.http, strings::NOT_A_BOT_ADMIN)
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
                        .say(&cmdctx.ctx.http, strings::NOT_A_CHANNEL_ADMIN)
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
        let bot_id = ctx.data::<UserData>().bot_id;
        let shutdown_signal = ctx
            .data::<UserData>()
            .shutdown_signal
            .load(Ordering::Acquire);
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
        let commands = ctx.data::<UserData>().prepared_commands.clone();
        // Wrap Serenity's context and message objects into a CommandContext
        // for access to convenience functions.
        let mut cmdctx = CommandContext::new(ctx, msg);
        // Is this a direct message to the bot?
        let is_dm = match cmdctx.is_dm().await {
            Ok(is_dm) => is_dm,
            Err(err) => {
                eprintln!(
                    "Could not figure out whether this message is a DM:\n{:#?}",
                    err
                );
                cmdctx
                    .msg
                    .channel_id
                    .say(&cmdctx.ctx.http, lib::strings::UNSPECIFIED_ERROR)
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
                &cmdctx.ctx.http,
                "Sorry, I can not help you right now. I am about to shut down!",
            );
            return;
        }
        // Poor man's try block
        let res: Result<(), lib::meetup::Error> = Self::handle_message(&mut cmdctx, commands).await;
        if let Err(err) = res {
            eprintln!("Error in message handler:\n{:#?}", err);
            let _ = cmdctx
                .msg
                .channel_id
                .say(&cmdctx.ctx.http, lib::strings::UNSPECIFIED_ERROR);
        }
    }

    // Set a handler to be called on the `ready` event. This is called when a
    // shard is booted, and a READY payload is sent by Discord. This payload
    // contains data like the current user's guild Ids, current user data,
    // private channels, and more.
    //
    // In this case, just print what the current user's username is.
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected!", ready.user.name);
    }

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
            [guild] => *guild,
            _ => {
                eprintln!("cache_ready event received not exactly one guild");
                return;
            }
        };
        let members = match ctx.cache.guild(guild_id).map(|guild| guild.members.clone()) {
            Some(members) => members,
            None => return,
        };
        println!("Updating {} cached member nicks", members.len());
        for member in members {
            let nick = member.nick.as_deref().unwrap_or(member.user.name.as_str());
            Self::update_member_nick(&ctx, member.user.id, nick)
                .await
                .ok();
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
                    .say(ctx.http(), "Yessir! Linking Meetup")
                    .await
                    .ok();
            }
            "unlink-meetup" => {
                interaction
                    .channel_id
                    .say(ctx.http(), "Un-linking Meetup")
                    .await
                    .ok();
            }
            _ => {
                interaction
                    .channel_id
                    .say(ctx.http(), "Unknown command")
                    .await
                    .ok();
            }
        };
    }
}

impl Handler {
    async fn send_welcome_message(ctx: &Context, user: &User) {
        user.direct_message(
            &ctx.http,
            CreateMessage::new().content(strings::WELCOME_MESSAGE),
        )
        .await
        .ok();
    }

    async fn update_member_nick(
        ctx: &Context,
        discord_id: UserId,
        nick: &str,
    ) -> Result<(), lib::meetup::Error> {
        let pool = ctx.data::<UserData>().pool.clone();
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
