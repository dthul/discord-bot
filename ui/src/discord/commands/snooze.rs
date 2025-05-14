use command_macro::command;

#[command]
#[regex(r"snooze\s+(?P<num_days>[0-9]+)\s*d(ay)?s?")]
#[level(admin)]
#[help(
    "snooze `X` days",
    "_(in game channel)_ snoozes reminders for _X_ days"
)]
fn snooze<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let num_days: u32 = captures
        .name("num_days")
        .expect("Regex capture does not contain 'num_days'")
        .as_str()
        .parse()
        .map(|num_days: u32| num_days.min(180))
        .map_err(|_err| simple_error::SimpleError::new("Invalid number of days specified"))?;
    // Check whether this is a game channel
    // TODO: make this a macro
    let is_game_channel: bool = context.is_game_channel(None).await?;
    if !is_game_channel {
        context
            .msg
            .channel_id
            .say(&context.ctx.http, lib::strings::CHANNEL_NOT_BOT_CONTROLLED)
            .await
            .ok();
        return Ok(());
    };
    let pool = context.pool();
    if num_days == 0 {
        // Remove the snooze
        sqlx::query!(
            r#"UPDATE event_series_text_channel SET snooze_until = NULL WHERE discord_id = $1"#,
            context.msg.channel_id.get() as i64
        )
        .execute(&pool)
        .await?;
        context
            .msg
            .channel_id
            .say(&context.ctx.http, "Disabled snoozing.")
            .await
            .ok();
    } else {
        let snooze_until = chrono::Utc::now() + chrono::Duration::days(num_days as i64);
        // Set a new snooze date
        sqlx::query!(
            r#"UPDATE event_series_text_channel SET snooze_until = $2 WHERE discord_id = $1"#,
            context.msg.channel_id.get() as i64,
            snooze_until
        )
        .execute(&pool)
        .await?;
        context
            .msg
            .channel_id
            .say(&context.ctx.http, format!("Snoozing for {} days.", num_days))
            .await
            .ok();
    }
    Ok(())
}
