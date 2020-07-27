use command_macro::command;
use lib::discord::CacheAndHttp;
use redis::AsyncCommands;
use serenity::model::{channel::PermissionOverwriteType, id::UserId, permissions::Permissions};

#[command]
#[regex(r"add\s+{mention_pattern}", mention_pattern)]
#[level(host)]
#[help(
    "add `@some-user`",
    "_(in game channel or managed channel)_ adds a user to the channel."
)]
fn add_user<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Get the Discord ID of the user that is supposed to
    // be added to the channel
    let discord_id = captures.name("mention_id").unwrap().as_str();
    // Try to convert the specified ID to an integer
    let discord_id = match discord_id.parse::<u64>() {
        Ok(id) => id,
        _ => {
            context
                .msg
                .channel_id
                .say(&context.ctx, lib::strings::CHANNEL_ADD_USER_INVALID_DISCORD)
                .await
                .ok();
            return Ok(());
        }
    };
    channel_add_or_remove_user_impl(
        context, discord_id, /*add*/ true, /*as_host*/ false,
    )
    .await
}

#[command]
#[regex(r"add\s*host\s+{mention_pattern}", mention_pattern)]
#[level(host)]
#[help(
    "add host `@some-user`",
    "_(in game channel or managed channel)_ makes a user an additional Host. _(Desktop only)_"
)]
fn add_host<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Get the Discord ID of the user that is supposed to
    // be added to the channel
    let discord_id = captures.name("mention_id").unwrap().as_str();
    // Try to convert the specified ID to an integer
    let discord_id = match discord_id.parse::<u64>() {
        Ok(id) => id,
        _ => {
            context
                .msg
                .channel_id
                .say(&context.ctx, lib::strings::CHANNEL_ADD_USER_INVALID_DISCORD)
                .await
                .ok();
            return Ok(());
        }
    };
    channel_add_or_remove_user_impl(
        context, discord_id, /*add*/ true, /*as_host*/ true,
    )
    .await
}

#[command]
#[regex(r"remove\s+{mention_pattern}", mention_pattern)]
#[level(host)]
#[help(
    "remove `@some-user`",
    "_(in game channel or managed channel)_ removes a user from the channel."
)]
fn remove_user<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Get the Discord ID of the user that is supposed to
    // be added to the channel
    let discord_id = captures.name("mention_id").unwrap().as_str();
    // Try to convert the specified ID to an integer
    let discord_id = match discord_id.parse::<u64>() {
        Ok(id) => id,
        _ => {
            context
                .msg
                .channel_id
                .say(&context.ctx, lib::strings::CHANNEL_ADD_USER_INVALID_DISCORD)
                .await
                .ok();
            return Ok(());
        }
    };
    channel_add_or_remove_user_impl(
        context, discord_id, /*add*/ false, /*as_host*/ false,
    )
    .await
}

#[command]
#[regex(r"remove\s*host\s+{mention_pattern}", mention_pattern)]
#[level(host)]
#[help(
    "remove host `@some-user`",
    "_(in game channel or managed channel)_ makes a user no longer a Host."
)]
fn remove_host<'a>(
    context: &'a mut super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Get the Discord ID of the user that is supposed to
    // be added to the channel
    let discord_id = captures.name("mention_id").unwrap().as_str();
    // Try to convert the specified ID to an integer
    let discord_id = match discord_id.parse::<u64>() {
        Ok(id) => id,
        _ => {
            context
                .msg
                .channel_id
                .say(&context.ctx, lib::strings::CHANNEL_ADD_USER_INVALID_DISCORD)
                .await
                .ok();
            return Ok(());
        }
    };
    channel_add_or_remove_user_impl(
        context, discord_id, /*add*/ false, /*as_host*/ true,
    )
    .await
}

async fn channel_add_or_remove_user_impl(
    context: &mut super::CommandContext,
    discord_id: u64,
    add: bool,
    as_host: bool,
) -> Result<(), lib::meetup::Error> {
    // Check whether this is a bot controlled channel
    let is_game_channel = context.is_game_channel().await?;
    let is_managed_channel = context.is_managed_channel().await?;
    let is_bot_admin = context.is_admin().await?;
    // Only bot admins can add/remove hosts
    if !is_bot_admin && as_host {
        context
            .msg
            .channel_id
            .say(&context.ctx, lib::strings::NOT_A_BOT_ADMIN)
            .await
            .ok();
        return Ok(());
    }
    // Managed channels and hosts don't use roles but user-specific permission overwrites
    let discord_api: CacheAndHttp = Into::into(&context.ctx);
    if is_game_channel && !is_managed_channel {
        let channel_roles = lib::get_channel_roles(
            context.msg.channel_id.0,
            context.async_redis_connection().await?,
        )
        .await?;
        let channel_roles = match channel_roles {
            Some(roles) => roles,
            None => {
                context
                    .msg
                    .channel_id
                    .say(&context.ctx, lib::strings::CHANNEL_NOT_BOT_CONTROLLED)
                    .await
                    .ok();
                return Ok(());
            }
        };
        // Only bot admins can add users
        if !is_bot_admin && add {
            context
                .msg
                .channel_id
                .say(&context.ctx, lib::strings::NOT_A_BOT_ADMIN)
                .await
                .ok();
            return Ok(());
        }
        // Figure out whether there is a voice channel
        let voice_channel_id = match lib::get_channel_voice_channel(
            context.msg.channel_id,
            context.async_redis_connection().await?,
        )
        .await
        {
            Ok(id) => id,
            Err(err) => {
                eprintln!(
                    "Could not figure out whether this channel has a voice channel:\n{:#?}",
                    err
                );
                None
            }
        };
        if add {
            // Try to add the user to the channel
            match context
                .ctx
                .http
                .add_member_role(
                    lib::discord::sync::ids::GUILD_ID.0,
                    discord_id,
                    channel_roles.user,
                )
                .await
            {
                Ok(()) => {
                    context.msg.react(&context.ctx, '\u{2705}').await.ok();
                    context
                        .msg
                        .channel_id
                        .say(&context.ctx, format!("Welcome <@{}>!", discord_id))
                        .await
                        .ok();
                }
                Err(err) => {
                    eprintln!("Could not assign channel role: {}", err);
                    context
                        .msg
                        .channel_id
                        .say(&context.ctx, lib::strings::CHANNEL_ROLE_ADD_ERROR)
                        .await
                        .ok();
                }
            }
            if as_host {
                // Grant direct permissions
                let new_permissions = Permissions::READ_MESSAGES
                    | Permissions::MENTION_EVERYONE
                    | Permissions::MANAGE_MESSAGES;
                match lib::discord::add_channel_user_permissions(
                    &discord_api,
                    context.msg.channel_id,
                    UserId(discord_id),
                    new_permissions,
                )
                .await
                {
                    Err(err) => {
                        eprintln!("Could not assign channel permissions:\n{:#?}", err);
                        context
                            .msg
                            .channel_id
                            .say(
                                &context.ctx,
                                "Something went wrong assigning the channel permissions",
                            )
                            .await
                            .ok();
                    }
                    Ok(true) => {
                        context
                            .msg
                            .channel_id
                            .say(
                                &context.ctx,
                                lib::strings::CHANNEL_ADDED_NEW_HOST(discord_id),
                            )
                            .await
                            .ok();
                    }
                    Ok(false) => (),
                }
                // Also grant permissions in a possibly existing voice channel
                if let Some(voice_channel_id) = voice_channel_id {
                    let new_permissions = Permissions::READ_MESSAGES
                        | Permissions::CONNECT
                        | Permissions::MUTE_MEMBERS
                        | Permissions::DEAFEN_MEMBERS
                        | Permissions::MOVE_MEMBERS
                        | Permissions::PRIORITY_SPEAKER;
                    if let Err(err) = lib::discord::add_channel_user_permissions(
                        &discord_api,
                        voice_channel_id,
                        UserId(discord_id),
                        new_permissions,
                    )
                    .await
                    {
                        eprintln!("Could not assign voice channel permissions:\n{:#?}", err);
                        context
                            .msg
                            .channel_id
                            .say(
                                &context.ctx,
                                "Something went wrong assigning the voice channel permissions",
                            )
                            .await
                            .ok();
                    }
                }
            }
        } else {
            // Try to remove the user from the channel
            if let Some(host_role) = channel_roles.host {
                if let Err(err) = context
                    .ctx
                    .http
                    .remove_member_role(lib::discord::sync::ids::GUILD_ID.0, discord_id, host_role)
                    .await
                {
                    eprintln!("Could not remove host channel role:\n{:#?}", err);
                    context
                        .msg
                        .channel_id
                        .say(&context.ctx, lib::strings::CHANNEL_ROLE_REMOVE_ERROR)
                        .await
                        .ok();
                }
            }
            if as_host {
                // Reduce direct permissions
                let permissions_to_remove =
                    Permissions::MANAGE_MESSAGES | Permissions::MENTION_EVERYONE;
                if let Err(err) = lib::discord::remove_channel_user_permissions(
                    &discord_api,
                    context.msg.channel_id,
                    UserId(discord_id),
                    permissions_to_remove,
                )
                .await
                {
                    eprintln!("Could not reduce channel permissions:\n{:#?}", err);
                    context
                        .msg
                        .channel_id
                        .say(
                            &context.ctx,
                            "Something went wrong reducing the channel permissions",
                        )
                        .await
                        .ok();
                }
                // Also reduce direct permissions in a possibly existing voice channel
                if let Some(voice_channel_id) = voice_channel_id {
                    let permissions_to_remove = Permissions::MUTE_MEMBERS
                        | Permissions::DEAFEN_MEMBERS
                        | Permissions::MOVE_MEMBERS
                        | Permissions::PRIORITY_SPEAKER;
                    if let Err(err) = lib::discord::remove_channel_user_permissions(
                        &discord_api,
                        voice_channel_id,
                        UserId(discord_id),
                        permissions_to_remove,
                    )
                    .await
                    {
                        eprintln!("Could not reduce voice channel permissions:\n{:#?}", err);
                        context
                            .msg
                            .channel_id
                            .say(
                                &context.ctx,
                                "Something went wrong reducing the voice channel permissions",
                            )
                            .await
                            .ok();
                    }
                }
            } else {
                // Remove user completely
                // Remove direct permissions
                if let Err(err) = context
                    .msg
                    .channel_id
                    .delete_permission(
                        &context.ctx,
                        PermissionOverwriteType::Member(UserId(discord_id)),
                    )
                    .await
                {
                    eprintln!("Could not remove channel permissions:\n{:#?}", err);
                    context
                        .msg
                        .channel_id
                        .say(
                            &context.ctx,
                            "Something went wrong revoking the channel permissions",
                        )
                        .await
                        .ok();
                }
                // Also remove permissions from a possibly existing voice channel
                if let Some(voice_channel_id) = voice_channel_id {
                    if let Err(err) = voice_channel_id
                        .delete_permission(
                            &context.ctx,
                            PermissionOverwriteType::Member(UserId(discord_id)),
                        )
                        .await
                    {
                        eprintln!("Could not revoke voice channel permissions:\n{:#?}", err);
                        context
                            .msg
                            .channel_id
                            .say(
                                &context.ctx,
                                "Something went wrong revoking the voice channel permissions",
                            )
                            .await
                            .ok();
                    }
                }
                match context
                    .ctx
                    .http
                    .remove_member_role(
                        lib::discord::sync::ids::GUILD_ID.0,
                        discord_id,
                        channel_roles.user,
                    )
                    .await
                {
                    Err(err) => {
                        eprintln!("Could not remove channel role: {}", err);
                        context
                            .msg
                            .channel_id
                            .say(&context.ctx, lib::strings::CHANNEL_ROLE_REMOVE_ERROR)
                            .await
                            .ok();
                    }
                    _ => (),
                }
            }
            context.msg.react(&context.ctx, '\u{2705}').await.ok();
            // Remember which users were removed manually
            if as_host {
                let redis_channel_removed_hosts_key =
                    format!("discord_channel:{}:removed_hosts", context.msg.channel_id.0);
                context
                    .async_redis_connection()
                    .await?
                    .sadd(redis_channel_removed_hosts_key, discord_id)
                    .await?;
            } else {
                let redis_channel_removed_users_key =
                    format!("discord_channel:{}:removed_users", context.msg.channel_id.0);
                context
                    .async_redis_connection()
                    .await?
                    .sadd(redis_channel_removed_users_key, discord_id)
                    .await?;
            }
        }
    } else if is_managed_channel && !is_game_channel {
        if add {
            let new_permissions = if as_host {
                // Add normal and host specific permissions
                Permissions::READ_MESSAGES
                    | Permissions::MANAGE_MESSAGES
                    | Permissions::MENTION_EVERYONE
            } else {
                // Add only user permissions
                Permissions::READ_MESSAGES
            };
            let permissions_changed = lib::discord::add_channel_user_permissions(
                &discord_api,
                context.msg.channel_id,
                UserId(discord_id),
                new_permissions,
            )
            .await?;
            if permissions_changed {
                context
                    .msg
                    .channel_id
                    .say(&context.ctx, format!("Welcome <@{}>!", discord_id))
                    .await
                    .ok();
            }
            context.msg.react(&context.ctx, '\u{2705}').await.ok();
        } else {
            // Assume that users with the READ_MESSAGES, MANAGE_MESSAGES and
            // MENTION_EVERYONE permission are channel hosts
            let target_is_host = lib::discord::is_host(
                &discord_api,
                context.msg.channel_id,
                UserId(discord_id),
                context.async_redis_connection().await?,
            )
            .await?;
            if target_is_host && !is_bot_admin {
                context
                    .msg
                    .channel_id
                    .say(&context.ctx, lib::strings::NOT_A_BOT_ADMIN)
                    .await
                    .ok();
                return Ok(());
            }
            let permissions_to_remove = if as_host {
                // Remove only host specific permissions
                Permissions::MANAGE_MESSAGES | Permissions::MENTION_EVERYONE
            } else {
                // Remove host and user permissions
                Permissions::READ_MESSAGES
                    | Permissions::MANAGE_MESSAGES
                    | Permissions::MENTION_EVERYONE
            };
            lib::discord::remove_channel_user_permissions(
                &discord_api,
                context.msg.channel_id,
                UserId(discord_id),
                permissions_to_remove,
            )
            .await?;
            context.msg.react(&context.ctx, '\u{2705}').await.ok();
        }
    }
    Ok(())
}
