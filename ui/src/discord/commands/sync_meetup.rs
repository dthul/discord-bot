use command_macro::command;
use futures_util::TryFutureExt;
use std::time::Duration;

#[command]
#[regex(r"sync\s*meetup")]
#[level(admin)]
#[help(
    "sync meetup",
    "Synchronizes the database with the current Meetup data. Please use carefully due to Meetup \
     API usage limits."
)]
fn sync_meetup<'a>(
    context: &'a mut super::CommandContext,
    _: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Send the syncing future to the executor
    let pool = context.pool();
    let async_meetup_client = context.meetup_client();
    let sync_task = {
        let task = async move {
            lib::meetup::sync::sync_task(async_meetup_client, &pool)
                .await
                .map(|_| ())
        };
        // Wrap the task in a timeout
        tokio::time::timeout(
            Duration::from_secs(5 * 60),
            task.unwrap_or_else(|err| {
                eprintln!("Syncing task failed: {}", err);
            }),
        )
        .unwrap_or_else(|err| {
            eprintln!("Syncing task timed out: {}", err);
        })
    };
    tokio::spawn(sync_task);
    context
        .msg
        .channel_id
        .say(
            &context.ctx.http,
            "Started asynchronous Meetup synchronization task",
        )
        .await
        .ok();
    Ok(())
}
