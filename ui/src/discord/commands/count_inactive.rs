use command_macro::command;
use serenity::all::CacheHttp;

#[command]
#[regex(r"count\s*inactive")]
#[level(admin)]
#[help("count inactive", "returns the number of members without any role")]
fn count_inactive<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let num_inactive_users = lib::discord::sync::ids::GUILD_ID
        .to_guild_cached(&context.ctx.cache)
        .map(|guild| {
            guild
                .members
                .iter()
                .filter(|member| member.roles.is_empty())
                .count()
        });
    if let Some(num_inactive_users) = num_inactive_users {
        context
            .msg
            .channel_id
            .say(
                context.ctx.http(),
                format!("There are {} users without any role", num_inactive_users),
            )
            .await
            .ok();
    } else {
        context
            .msg
            .channel_id
            .say(context.ctx.http(), "Could not find the guild")
            .await
            .ok();
        return Ok(());
    };
    Ok(())
}

#[command]
#[regex(r"count\s*members")]
#[level(admin)]
#[help("count members", "returns the number of members")]
fn count_members<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let num_members = lib::discord::sync::ids::GUILD_ID
        .to_guild_cached(&context.ctx.cache)
        .map(|guild| guild.members.len());
    if let Some(num_members) = num_members {
        context
            .msg
            .channel_id
            .say(
                context.ctx.http(),
                format!("There are {} members", num_members),
            )
            .await
            .ok();
    } else {
        context
            .msg
            .channel_id
            .say(context.ctx.http(), "Could not find the guild")
            .await
            .ok();
    }
    Ok(())
}
