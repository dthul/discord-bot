use command_macro::command;
use lib::end_adventure::EndAdventureResult;
use serenity::{all::ChannelId, model::channel::Channel};

#[command]
#[regex(r"end\s*all")]
#[level(admin)]
#[help("end all", "ends all expired adventure channels")]
fn end_all<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let pool = context.pool();
    // Find all channels which can be ended
    let channel_ids = sqlx::query!(
        r#"SELECT discord_id FROM event_series_text_channel WHERE
        deleted IS NULL AND
        expiration_time < $1 AND
        (snooze_until IS NULL OR snooze_until < $1) AND
        deletion_time IS NULL"#,
        chrono::Utc::now(),
    )
    .map(|row| ChannelId::new(row.discord_id as u64))
    .fetch_all(&pool)
    .await?;
    if channel_ids.is_empty() {
        context
            .msg
            .channel_id
            .say(
                &context.ctx,
                "Found no adventure channels that can be ended",
            )
            .await?;
        return Ok(());
    } else {
        context
            .msg
            .channel_id
            .say(
                &context.ctx,
                format!(
                    "Found {} adventure channels that can possibly be ended, hang on...",
                    channel_ids.len()
                ),
            )
            .await?;
    }
    for channel_id in channel_ids {
        let mut tx = pool.begin().await?;
        let end_adventure_result = lib::end_adventure::end_adventure(channel_id, &mut tx).await?;
        tx.commit().await?;
        match end_adventure_result {
            EndAdventureResult::NewlyMarkedForDeletion(_) => channel_id
                .say(&context.ctx.http, lib::strings::CHANNEL_MARKED_FOR_CLOSING)
                .await
                .ok(),
            EndAdventureResult::NotAGameChannel
            | EndAdventureResult::NoExpirationTime
            | EndAdventureResult::NotYetExpired
            | EndAdventureResult::AlreadyMarkedForDeletion(_) => continue,
        };
        let channel = channel_id.to_guild_channel(&context.ctx).await;
        let channel_name = match &channel {
            Ok(Channel::Guild(channel)) => &channel.name,
            _ => "'unknown'",
        };
        context
            .msg
            .channel_id
            .say(
                &context.ctx,
                lib::strings::CHANNEL_MARKED_FOR_CLOSING_ALERT(
                    channel_id,
                    channel_name,
                    context.msg.author.id,
                ),
            )
            .await
            .ok();
        if let Some(bot_alerts_channel_id) = lib::discord::sync::ids::BOT_ALERTS_CHANNEL_ID {
            if bot_alerts_channel_id != context.msg.channel_id {
                bot_alerts_channel_id
                    .say(
                        &context.ctx,
                        lib::strings::CHANNEL_MARKED_FOR_CLOSING_ALERT(
                            channel_id,
                            channel_name,
                            context.msg.author.id,
                        ),
                    )
                    .await
                    .ok();
            }
        }
    }
    Ok(())
}
