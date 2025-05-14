use command_macro::command;

#[command]
#[regex(r"clone\s+event\s+(?P<meetup_event_id>[^\s]+)")]
#[level(admin)]
fn clone_event<'a>(
    context: super::CommandContext,
    captures: regex::Captures<'a>,
) -> super::CommandResult<'a> {
    // Get the mentioned Meetup event
    let meetup_event_id = captures.name("meetup_event_id").unwrap().as_str();
    let meetup_client_option = context.meetup_client()?.clone();
    let async_meetup_client = meetup_client_option
        .lock()
        .await
        .clone()
        .ok_or_else(|| simple_error::SimpleError::new("Meetup client not available"))?;
    let urlname = "SwissRPG-Zurich";
    let new_event_hook = Box::new(|new_event: lib::meetup::api::NewEvent| {
        let date_time = new_event.time.clone();
        lib::flow::ScheduleSessionFlow::new_event_hook(new_event, date_time, meetup_event_id, false)
    });
    let new_event = lib::meetup::util::clone_event(
        urlname,
        meetup_event_id,
        async_meetup_client.as_ref(),
        Some(new_event_hook),
    )
    .await?;
    // Try to transfer the RSVPs to the new event
    let redis_client = context.redis_client();
    let mut redis_connection = redis_client.await?.get_multiplexed_async_connection().await?;
    if let Err(_) = lib::meetup::util::clone_rsvps(
        urlname,
        meetup_event_id,
        &new_event.id,
        &mut redis_connection,
        async_meetup_client.as_ref(),
        context.oauth2_consumer()?,
    )
    .await
    {
        let _ = context
            .msg
            .channel_id
            .say(context.ctx, "Could not transfer all RSVPs to the new event");
    }
    context.msg.react(context.ctx, '\u{2705}').await.ok();
    context
        .msg
        .channel_id
        .say(
            context.ctx,
            format!("Created new Meetup event: {}", new_event.link),
        )
        .await
        .ok();
    Ok(())
}
