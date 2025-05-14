use std::{ops::Deref, sync::Arc};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Extension},
    http::StatusCode,
    routing::post,
    Router,
};
use axum_extra::{headers::Header, TypedHeader};
use lazy_static::lazy_static;

use super::server::State;

pub fn create_routes() -> Router {
    Router::new().route(
        "/webhooks/stripe",
        post(stripe_webhook_handler).layer(DefaultBodyLimit::max(32768)),
    )
}

struct StripeSignatureHeader(String);

lazy_static! {
    static ref STRIPE_SIGNATURE_HEADER: axum_extra::headers::HeaderName =
        axum_extra::headers::HeaderName::from_lowercase(b"stripe-signature").unwrap();
}

impl Deref for StripeSignatureHeader {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Header for StripeSignatureHeader {
    fn name() -> &'static axum_extra::headers::HeaderName {
        &STRIPE_SIGNATURE_HEADER
    }

    fn decode<'i, I>(values: &mut I) -> Result<Self, axum_extra::headers::Error>
    where
        Self: Sized,
        I: Iterator<Item = &'i axum_extra::headers::HeaderValue>,
    {
        let value = values
            .next()
            .ok_or_else(axum_extra::headers::Error::invalid)?;
        let value = value
            .to_str()
            .map_err(|_| axum_extra::headers::Error::invalid())?;
        Ok(StripeSignatureHeader(value.into()))
    }

    fn encode<E: Extend<axum_extra::headers::HeaderValue>>(&self, values: &mut E) {
        match axum_extra::headers::HeaderValue::from_str(&self.0) {
            Ok(header_value) => values.extend(Some(header_value)),
            Err(err) => eprintln!("Failed to encode Stripe-Signature HTTP header: {:#?}", err),
        }
    }
}

async fn stripe_webhook_handler(
    TypedHeader(signature): TypedHeader<StripeSignatureHeader>,
    Extension(state): Extension<Arc<State>>,
    payload: Bytes,
) -> StatusCode {
    println!("Stripe webhook!");
    let stripe_webhook_secret =
        if let Some(stripe_webhook_secret) = state.stripe_webhook_secret.clone() {
            stripe_webhook_secret
        } else {
            eprintln!("Stripe webhook secret not set");
            return StatusCode::INTERNAL_SERVER_ERROR;
        };
    let event = if let Ok(payload) = std::str::from_utf8(&payload) {
        if let Ok(event) =
            stripe::Webhook::construct_event(&payload, &signature, &stripe_webhook_secret)
        {
            println!("Webhook event:\n{:#?}", event);
            event
        } else {
            eprintln!("Event construction failed");
            return StatusCode::BAD_REQUEST;
        }
    } else {
        eprintln!("Payload to UTF8 conversion failed");
        return StatusCode::BAD_REQUEST;
    };
    let webhook_handler_future = async move {
        if event.type_ == stripe::EventType::CustomerSubscriptionCreated {
            if let stripe::EventObject::Subscription(subscription) = event.data.object {
                if let Err(err) = handle_new_subscription(
                    &state.discord_cache_http,
                    &state.stripe_client,
                    &subscription,
                )
                .await
                {
                    eprintln!("Could not handle new subscription event:\n{:#?}", err);
                }
            }
        }
    };
    tokio::spawn(webhook_handler_future);
    return StatusCode::OK;
}

async fn handle_new_subscription(
    discord_api: &lib::discord::CacheAndHttp,
    stripe_client: &stripe::Client,
    subscription: &stripe::Subscription,
) -> Result<(), lib::meetup::Error> {
    // let customer = match &subscription.customer {
    //     stripe::Expandable::Object(customer) => *customer.clone(),
    //     stripe::Expandable::Id(customer_id) => {
    //         let customer = stripe::Customer::retrieve(stripe_client, customer_id, &[]).await?;
    //         customer
    //     }
    // };
    let (customer, product) =
        lib::tasks::subscription_roles::get_customer_and_product(stripe_client, subscription)
            .await?;
    println!(
        "Received new subscription with product '{:#?}'",
        product.name
    );
    // Wait for a few seconds and re-query the customer object from Stripe,
    // since the webhook is sometimes faster than the customer metadata is updated
    tokio::time::sleep(tokio::time::Duration::from_secs(20)).await;
    let customer = stripe::Customer::retrieve(stripe_client, &customer.id, &[]).await?;
    if let Some(username) = customer
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("Discord"))
    {
        // Try to find the Discord user associated with this subscription
        let id =
            lib::tasks::subscription_roles::discord_username_to_id(discord_api, username).await?;
        if let Some(discord_id) = id {
            // TODO: might block
            let discord_user = discord_id.to_user(discord_api).await?;
            let is_champion_product = product.name.as_ref().map_or(false, |name| {
                lib::tasks::subscription_roles::CHAMPION_PRODUCT_REGEX.is_match(name)
            });
            let is_insider_product = product.name.as_ref().map_or(false, |name| {
                lib::tasks::subscription_roles::INSIDER_PRODUCT_REGEX.is_match(name)
            });
            if is_champion_product {
                if let Ok(true) = discord_user
                    .has_role(
                        discord_api,
                        lib::discord::sync::ids::GUILD_ID,
                        lib::discord::sync::ids::GAME_MASTER_ID,
                    )
                    .await
                {
                    println!("Adding GM Champion role");
                    lib::tasks::subscription_roles::add_member_role(
                        discord_api,
                        discord_id,
                        lib::tasks::subscription_roles::ids::GM_CHAMPION_ID,
                        Some(
                            "Automatic role assignment due to being a GM champion (via Stripe \
                             Webhook)",
                        ),
                    )
                    .await?;
                } else {
                    println!("Adding Champion role");
                    lib::tasks::subscription_roles::add_member_role(
                        discord_api,
                        discord_id,
                        lib::tasks::subscription_roles::ids::CHAMPION_ID,
                        Some(
                            "Automatic role assignment due to being a champion (via Stripe \
                             Webhook)",
                        ),
                    )
                    .await?;
                }
            }
            if is_insider_product {
                println!("Adding Insider role");
                lib::tasks::subscription_roles::add_member_role(
                    discord_api,
                    discord_id,
                    lib::tasks::subscription_roles::ids::INSIDER_ID,
                    Some("Automatic role assignment due to being an insider (via Stripe Webhook)"),
                )
                .await?;
            }
        } else {
            eprintln!(
                "Could not match the Discord username '{}' to an actual Discord user",
                username
            );
        }
    } else {
        eprintln!("Found no 'Discord' field on the customer metadata object");
    }
    Ok(())
}
