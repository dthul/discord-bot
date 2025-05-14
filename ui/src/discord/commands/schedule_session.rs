use command_macro::command;
use serenity::builder::CreateMessage;

#[command]
#[regex(r"schedule\s*session")]
#[level(host)]
#[help(
    "schedule session",
    "allows you to schedule a new session for your adventure."
)]
fn schedule_session<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Find the series belonging to the channel
    let pool = context.pool();
    let mut tx = pool.begin().await?;
    let event_series = lib::get_channel_series(context.msg.channel_id, &mut tx).await?;
    let event_series = if let Some(event_series) = event_series {
        event_series
    } else {
        context
            .msg
            .channel_id
            .say(&context.ctx.http, lib::strings::CHANNEL_NOT_BOT_CONTROLLED)
            .await
            .ok();
        return Ok(());
    };
    // Create a new Flow
    let flow =
        lib::flow::ScheduleSessionFlow::new(context.async_redis_connection().await?, event_series)
            .await?;
    let link = format!("{}/schedule_session/{}", lib::urls::BASE_URL, flow.id);
    context
        .msg
        .author
        .direct_message(
            &context.ctx.http,
            CreateMessage::new().content(format!(
                "Use the following link to schedule your next session:\n{}",
                link
            )),
        )
        .await
        .ok();
    context.msg.react(&context.ctx.http, '\u{2705}').await.ok();
    Ok(())
}
