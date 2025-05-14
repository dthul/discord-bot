use command_macro::command;
use lib::end_adventure::EndAdventureResult;
use serenity::{all::CacheHttp, model::channel::Channel};

#[command]
#[regex(r"end\s*adventure")]
#[level(host)]
#[help(
    "end adventure",
    "sets the channel for closure at the end of an adventure. The channel won't be deleted \
     immediately but within 24 hours."
)]
fn end_adventure<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let pool = context.pool();
    let mut tx = pool.begin().await?;
    let end_adventure_result =
        lib::end_adventure::end_adventure(context.msg.channel_id, &mut tx).await?;
    tx.commit().await?;
    match end_adventure_result {
        EndAdventureResult::NotAGameChannel => context
            .msg
            .channel_id
            .say(context.ctx.http(), lib::strings::CHANNEL_NOT_BOT_CONTROLLED)
            .await
            .ok(),
        EndAdventureResult::NoExpirationTime => context
            .msg
            .channel_id
            .say(context.ctx.http(), lib::strings::CHANNEL_NO_EXPIRATION)
            .await
            .ok(),
        EndAdventureResult::NotYetExpired => context
            .msg
            .channel_id
            .say(context.ctx.http(), lib::strings::CHANNEL_NOT_YET_CLOSEABLE)
            .await
            .ok(),
        EndAdventureResult::AlreadyMarkedForDeletion(_) => context
            .msg
            .channel_id
            .say(
                context.ctx.http(),
                lib::strings::CHANNEL_ALREADY_MARKED_FOR_CLOSING,
            )
            .await
            .ok(),
        EndAdventureResult::NewlyMarkedForDeletion(_) => context
            .msg
            .channel_id
            .say(context.ctx.http(), lib::strings::CHANNEL_MARKED_FOR_CLOSING)
            .await
            .ok(),
    };
    let channel = context.channel().await;
    let channel_name = match &channel {
        Ok(Channel::Guild(channel)) => &channel.name,
        _ => "'unknown'",
    };
    if let Some(bot_alerts_channel_id) = lib::discord::sync::ids::BOT_ALERTS_CHANNEL_ID {
        bot_alerts_channel_id
            .say(
                context.ctx.http(),
                lib::strings::CHANNEL_MARKED_FOR_CLOSING_ALERT(
                    context.msg.channel_id,
                    channel_name,
                    context.msg.author.id,
                ),
            )
            .await
            .ok();
    }
    Ok(())
}
