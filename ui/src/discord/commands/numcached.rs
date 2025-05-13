use command_macro::command;

#[command]
#[regex(r"numcached")]
#[level(admin)]
#[help("numcached", "Shows the number of known server members")]
pub fn numcached<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let num_cached_members = lib::discord::sync::ids::GUILD_ID
        .to_guild_cached(&context.ctx.cache)
        .map(|guild| guild.members.len());
    if let Some(num_cached_members) = num_cached_members {
        context
            .msg
            .channel_id
            .say(
                &context.ctx.http,
                format!(
                    "I have {} members cached for this guild",
                    num_cached_members
                ),
            )
            .await
            .ok();
    } else {
        context
            .msg
            .channel_id
            .say(
                &context.ctx.http,
                "No guild associated with this message (use the command from a guild channel \
                 instead of a direct message).",
            )
            .await
            .ok();
    }
    Ok(())
}
