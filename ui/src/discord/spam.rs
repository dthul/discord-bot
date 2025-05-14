use aho_corasick::{AhoCorasick, AhoCorasickBuilder};
use redis::AsyncCommands;
use serenity::model::id::ChannelId;
use std::sync::Arc;

use super::bot::UserData;

pub(super) type SpamList = Arc<(Vec<String>, AhoCorasick)>;
pub(super) struct GameChannelsList {
    channel_ids: Vec<ChannelId>,
    last_updated: std::time::Instant,
}

impl Default for GameChannelsList {
    fn default() -> Self {
        Self {
            channel_ids: vec![],
            last_updated: std::time::Instant::now() - std::time::Duration::from_secs(1000000),
        }
    }
}

pub async fn message_hook(
    cmdctx: &mut super::commands::CommandContext,
) -> Result<(), lib::meetup::Error> {
    let alert_channel_id = if let Some(channel_id) = lib::discord::sync::ids::BOT_ALERTS_CHANNEL_ID
    {
        channel_id
    } else {
        return Ok(());
    };
    // Ignore messages from game channels
    let game_channels = get_game_channels_list(cmdctx).await?;
    if let Some(channel_id) = cmdctx
        .msg
        .guild_channel(&cmdctx.ctx)
        .await
        .ok()
        .map(|channel| channel.id)
    {
        if game_channels.channel_ids.contains(&channel_id) {
            return Ok(());
        }
    }
    let spam_list = get_spam_list(cmdctx).await?;
    let (word_list, spam_matcher) = &*spam_list;
    if let Some(mat) = spam_matcher.find(&cmdctx.msg.content.as_str()) {
        let word = &word_list[mat.pattern()];
        let mut msg = serenity::utils::MessageBuilder::new().push_bold("Spam Alert ");
        if let Some(admin_role_id) = lib::discord::sync::ids::ADMIN_ROLE_ID {
            msg = msg.mention(&admin_role_id);
        }
        let msg = msg
            .push("\nTrigger: ")
            .push_line_safe(word.as_str())
            .push("User: ")
            .mention(&cmdctx.msg.author.id)
            .push("\nMessage: ")
            .push_line_safe(cmdctx.msg.content.as_str())
            .push(
                format!(
                    "https://discordapp.com/channels/{guild_id}/{channel_id}/{message_id}",
                    guild_id = lib::discord::sync::ids::GUILD_ID.get(),
                    channel_id = cmdctx.msg.channel_id.get(),
                    message_id = cmdctx.msg.id.get()
                )
                .as_str(),
            );
        alert_channel_id
            .widen()
            .say(&cmdctx.ctx.http, msg.build())
            .await?;
    }
    Ok(())
}

async fn get_spam_list(
    cmdctx: &mut super::commands::CommandContext,
) -> Result<SpamList, lib::meetup::Error> {
    if let Some(spam_list) = cmdctx.ctx.data::<UserData>().spam_list.get() {
        return Ok(spam_list.clone());
    }
    // There is no spam list in the data map yet -> query it and store it in the data map
    // Query the spam list from Redis
    let redis_connection = cmdctx.async_redis_connection().await?;
    let word_list: Vec<String> = redis_connection.lrange("spam_word_list", 0, -1).await?;
    let word_matcher = AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .build(&word_list)
        .expect("Failed to build the aho-corasick matcher");
    let spam_list = Arc::new((word_list, word_matcher));
    // We insert the new spam list into the user data
    // If another thread did that before us, we return its spam list, such that each thread is guaranteed to use the same
    let spam_list = cmdctx
        .ctx
        .data::<UserData>()
        .spam_list
        .get_or_init(move || spam_list)
        .clone();
    Ok(spam_list)
}

async fn get_game_channels_list(
    cmdctx: &mut super::commands::CommandContext,
) -> Result<Arc<GameChannelsList>, lib::meetup::Error> {
    // Check if the channels list is already in the data map and up-to-date
    let user_data = cmdctx.ctx.data::<UserData>();
    let games_list = user_data.games_list.read().await;
    if (std::time::Instant::now() - games_list.last_updated)
        < std::time::Duration::from_secs(5 * 60)
    {
        return Ok(games_list.clone());
    }
    drop(games_list);
    // There is no games list in the data map yet or it is outdated -> query it and store it in the data map
    // Query the channel list from the database
    let pool = cmdctx.pool();
    let channel_ids = sqlx::query!(r#"SELECT discord_id FROM event_series_text_channel"#)
        .map(|row| ChannelId::new(row.discord_id as u64))
        .fetch_all(&pool)
        .await?;
    let games_list = Arc::new(GameChannelsList {
        channel_ids,
        last_updated: std::time::Instant::now(),
    });
    // We insert the new spam list into the user data
    *cmdctx.ctx.data::<UserData>().games_list.write().await = games_list.clone();
    Ok(games_list)
}
