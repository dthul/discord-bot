use command_macro::command;
use serenity::model::{
    channel::{Channel, PermissionOverwrite, PermissionOverwriteType},
    permissions::Permissions,
};

#[command]
#[regex(r"manage\s*channel")]
#[level(admin)]
#[help(
    "manage channel",
    "Enable the `[add|remove] user` and `[add|remove] host` commands for this channel."
)]
fn manage_channel<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    let channel_id = context.msg.channel_id;
    // Step 1: Try to mark this channel as managed
    let pool = context.pool();
    let mut tx = pool.begin().await?;
    let is_game_channel = context.is_game_channel(Some(&mut tx)).await?;
    if is_game_channel {
        context
            .msg
            .channel_id
            .say(&context.ctx.http, "Can not manage this channel")
            .await
            .ok();
        return Ok(());
    }
    sqlx::query!(
        r#"INSERT INTO managed_channel (discord_id) VALUES ($1) ON CONFLICT DO NOTHING"#,
        channel_id.get() as i64
    )
    .execute(&mut *tx)
    .await?;
    let channel = match context.msg.channel(&context.ctx).await {
        Ok(Channel::Guild(channel)) => channel,
        Ok(_) => {
            context
                .msg
                .channel_id
                .say(&context.ctx.http, "Can not manage this channel")
                .await
                .ok();
            return Ok(());
        }
        Err(err) => {
            eprintln!("manage channel failed:\n{:#?}", err);
            context
                .msg
                .channel_id
                .say(
                    &context.ctx.http,
                    "Error when trying to manage this channel",
                )
                .await
                .ok();
            return Ok(());
        }
    };
    // Step 2: Grant the bot continued access to the channel
    let bot_id = context.bot_id();
    channel
        .create_permission(
            &context.ctx.http,
            PermissionOverwrite {
                allow: Permissions::VIEW_CHANNEL,
                deny: Permissions::empty(),
                kind: PermissionOverwriteType::Member(bot_id),
            },
            None,
        )
        .await?;
    // Step 3: Grant all current users access to the channel
    let mut current_channel_members = channel.members(&context.ctx.cache)?;
    for member in &mut current_channel_members {
        // Don't explicitly grant access to admins
        let is_admin = {
            if let Ok(member_permissions) = member.permissions(&context.ctx.cache) {
                member_permissions.administrator()
            } else {
                false
            }
        };
        if is_admin {
            continue;
        }
        channel
            .create_permission(
                &context.ctx.http,
                PermissionOverwrite {
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty(),
                    kind: PermissionOverwriteType::Member(member.user.id),
                },
                None,
            )
            .await?;
    }
    tx.commit().await?;
    context.msg.react(&context.ctx.http, '\u{2705}').await.ok();
    Ok(())
}
