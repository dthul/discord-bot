use command_macro::command;
use redis::{AsyncCommands, RedisResult};
use serenity::{builder::EditChannel, model::channel::Channel};
use std::time::Duration;

#[command]
#[regex(r"topic\s+(?P<topic>[^\s].*)")]
#[help(
    "topic `some topic`",
    "renames the user topic voice channel to the specified topic"
)]
fn set_voice_topic<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Indicate that something is happening
    let _typing_indicator = context.msg.channel_id.start_typing(&context.ctx.http);
    // Check if there is a user topic voice channel
    let voice_channel_id = if let Some(id) = lib::discord::sync::ids::USER_TOPIC_VOICE_CHANNEL_ID {
        id
    } else {
        context
            .msg
            .channel_id
            .say(&context.ctx, "No voice channel has been configured")
            .await?;
        return Ok(());
    };
    // Check whether the specified topic is ok
    let topic = if let Some(topic) = captures.name("topic") {
        topic.as_str()
    } else {
        context
            .msg
            .channel_id
            .say(&context.ctx, "I had trouble parsing the new topic")
            .await?;
        return Ok(());
    };
    let topic = topic.trim();
    let topic_len = topic.chars().count();
    if topic_len < 2 {
        context
            .msg
            .channel_id
            .say(
                &context.ctx,
                "That topic is too short, it needs to have at least 2 characters.",
            )
            .await?;
        return Ok(());
    }
    if topic_len > 100 {
        context
            .msg
            .channel_id
            .say(
                &context.ctx,
                "That topic is over 100 characters. Have you considered the benefits of conciseness?",
            )
            .await?;
        return Ok(());
    }
    // Check the channel's current title
    let mut voice_channel =
        if let Ok(Channel::Guild(channel)) = voice_channel_id.to_channel(&context.ctx).await {
            channel
        } else {
            context
                .msg
                .channel_id
                .say(
                    &context.ctx,
                    "I could not find the user topic voice channel :(",
                )
                .await?;
            return Ok(());
        };
    let has_default_name = voice_channel.name()
        == lib::tasks::user_topic_voice_channel::DEFAULT_USER_TOPIC_VOICE_CHANNEL_NAME;
    let is_empty = voice_channel.members(&context.ctx)?.is_empty();
    if !has_default_name && !is_empty {
        // Someone is already using the voice channel
        context
            .msg
            .channel_id
            .say(
                &context.ctx,
                "Sorry but it seems there is already a topic going on. Try again when the voice channel is empty.",
            )
            .await?;
        return Ok(());
    }
    // We are good to go!
    // Rename the channel.
    // We wrap this request in a timeout to not block on the rate limit.
    match tokio::time::timeout(
        Duration::from_secs(5),
        voice_channel.edit(&context.ctx, EditChannel::new().name(topic)),
    )
    .await
    {
        Err(_) => {
            // The timeout elapsed
            context
                .msg
                .channel_id
                .say(
                    &context.ctx,
                    "Hold your horses. A topic was introduced recently. Please wait 10 minutes before changing it again.",
                )
                .await?;
            return Ok(());
        }
        Ok(Err(err)) => {
            context
                .msg
                .channel_id
                .say(
                    &context.ctx,
                    "There was an error renaming the voice channel :(",
                )
                .await?;
            return Err(err.into());
        }
        Ok(Ok(_)) => (),
    }
    // Try to store the renaming time in Redis
    if let Ok(con) = context.async_redis_connection().await {
        let _: RedisResult<()> = con
            .set(
                "user_topic_voice_channel_topic_time",
                chrono::Utc::now().to_rfc3339(),
            )
            .await;
    }
    context
        .msg
        .channel_id
        .say(
            &context.ctx,
            format!("The voice channel is now yours! New topic: _{}_", topic),
        )
        .await?;
    Ok(())
}
