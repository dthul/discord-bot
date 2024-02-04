#[tracing::instrument(skip(client))]
pub async fn list_active_subscriptions(
    client: &stripe::Client,
) -> Result<Vec<stripe::Subscription>, crate::meetup::Error> {
    let params = stripe::ListSubscriptions {
        status: Some(stripe::SubscriptionStatusFilter::Active),
        ..Default::default()
    };
    let mut paginator = stripe::Subscription::list(client, &params)
        .await?
        .paginate(params);
    let mut all_subscriptions = vec![];
    loop {
        // This is an unnecessary copy of the data in the page but we can't take it out of the paginator before calling next()
        all_subscriptions.extend(paginator.page.data.iter().cloned());
        if paginator.page.has_more {
            paginator = paginator.next(client).await?;
        } else {
            break;
        }
    }
    Ok(all_subscriptions)
}
