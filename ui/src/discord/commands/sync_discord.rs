use command_macro::command;

#[command]
#[regex(r"sync\s*discord")]
#[level(admin)]
#[help("sync discord", "Synchronizes Discord with the database")]
fn sync_discord<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let mut redis_connection = context
        .redis_client()
        .get_multiplexed_async_connection()
        .await?;
    let pool = context.pool();
    let mut discord_api = (&context.ctx).into();
    let bot_id = context.bot_id();
    // Spawn the syncing task
    tokio::spawn(async move {
        lib::discord::sync::sync_discord(&mut redis_connection, &pool, &mut discord_api, bot_id)
            .await
    });
    context
        .msg
        .channel_id
        .say(&context.ctx.http, "Started Discord synchronization task")
        .await
        .ok();
    Ok(())
}
