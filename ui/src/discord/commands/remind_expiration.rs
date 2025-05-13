use command_macro::command;

#[command]
#[regex(r"remind\s*expiration")]
#[level(admin)]
#[help(
    "remind expiration",
    "runs the end-of-game task, reminding channels of their expiration and deleting closed ones"
)]
fn remind_expiration<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let pool = context.pool();
    let bot_id = context.bot_id();
    let mut discord_api: lib::discord::CacheAndHttp = (&context.ctx).into();
    // Spawn the end-of-game task
    tokio::spawn(async move {
        lib::tasks::end_of_game::end_of_game_task(&pool, &mut discord_api, bot_id).await
    });
    context
        .msg
        .channel_id
        .say(&context.ctx.http, "Started expiration reminder task")
        .await
        .ok();
    Ok(())
}
