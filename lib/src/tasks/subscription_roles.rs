use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use lazy_static::lazy_static;
use serenity::futures::StreamExt;
use serenity::{
    http::CacheHttp,
    model::id::{RoleId, UserId},
};

pub const CHAMPION_PRODUCT_PATTERN: &'static str =
    r"(?i).*(Novice|Apprentice|Adept|Master|Legendary).*";
pub const INSIDER_PRODUCT_PATTERN: &'static str = r"(?i).*(Apprentice|Adept|Master|Legendary).*";

lazy_static! {
    pub static ref CHAMPION_PRODUCT_REGEX: regex::Regex =
        regex::Regex::new(CHAMPION_PRODUCT_PATTERN).unwrap();
    pub static ref INSIDER_PRODUCT_REGEX: regex::Regex =
        regex::Regex::new(INSIDER_PRODUCT_PATTERN).unwrap();
}

#[cfg(feature = "bottest")]
pub mod ids {
    use super::*;
    // Test server:
    pub const CHAMPION_ID: RoleId = RoleId::new(670250507436294144);
    pub const INSIDER_ID: RoleId = RoleId::new(670250754422079488);
    pub const GM_CHAMPION_ID: RoleId = RoleId::new(671107703703207940);
}

#[cfg(not(feature = "bottest"))]
pub mod ids {
    use super::*;
    // SwissRPG server:
    pub const CHAMPION_ID: RoleId = RoleId::new(670197555166052362);
    pub const INSIDER_ID: RoleId = RoleId::new(670201953883783169);
    pub const GM_CHAMPION_ID: RoleId = RoleId::new(671111220119470093);
}

pub async fn stripe_subscriptions_refresh_task(
    discord_api: crate::discord::CacheAndHttp,
    stripe_client: Arc<stripe::Client>,
) -> ! {
    // Sync every 8 hours, starting in an hour from now
    let mut interval_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + tokio::time::Duration::from_secs(60 * 60),
        tokio::time::Duration::from_secs(8 * 60 * 60),
    );
    // Run forever
    loop {
        // Wait for the next interval tick
        interval_timer.tick().await;
        println!("Refreshing Stripe subscription information");
        let join_handle = {
            let discord_api = discord_api.clone();
            let stripe_client = stripe_client.clone();
            tokio::spawn(async move { update_roles(&discord_api, &stripe_client).await })
        };
        match join_handle.await {
            Err(err) => {
                eprintln!("Stripe subscription update task failed:\n{:#?}", err);
            }
            Ok(Err(err)) => {
                eprintln!("Stripe subscription update task failed:\n{:#?}", err);
            }
            Ok(Ok(())) => {
                // Nothing to do
            }
        }
    }
}

pub async fn update_roles(
    discord_api: &crate::discord::CacheAndHttp,
    stripe_client: &stripe::Client,
) -> Result<(), crate::meetup::Error> {
    // Get all active subscriptions from Stripe
    let subscriptions = crate::stripe::list_active_subscriptions(stripe_client).await?;
    // For each subscription, find which product and customer are associated with it
    // and add the Discord name stored in Stripe to the appropriate list
    let mut new_champions = vec![];
    let mut new_insiders = vec![];
    for subscription in &subscriptions {
        // Since we don't have try blocks yet we need to match on every single error...
        let (customer, product) = match get_customer_and_product(stripe_client, subscription).await
        {
            Ok(customer_product) => customer_product,
            Err(err) => {
                eprintln!(
                    "Error in update_roles get_customer_and_product:\n{:#?}",
                    err
                );
                continue;
            }
        };
        let discord_id =
            match ensure_customer_has_discord_id(&customer, stripe_client, discord_api).await {
                Ok(discord_id) => discord_id,
                Err(err) => {
                    eprintln!(
                        "Error in update_roles ensure_customer_has_discord_id:\n{:#?}",
                        err
                    );
                    continue;
                }
            };
        if let Some(discord_id) = discord_id {
            let is_champion_product = product
                .name
                .as_ref()
                .map_or(false, |name| CHAMPION_PRODUCT_REGEX.is_match(name));
            let is_insider_product = product
                .name
                .as_ref()
                .map_or(false, |name| INSIDER_PRODUCT_REGEX.is_match(name));
            if is_champion_product {
                new_champions.push(discord_id);
            }
            if is_insider_product {
                new_insiders.push(discord_id);
            }
        } else {
            eprintln!(
                "Could not find Discord ID for Stripe customer {} ({:?})",
                customer.id, customer.email
            );
        }
    }
    // Now, check which Discord users already have the Champion and Insider roles
    let mut current_champions = vec![];
    let mut current_gm_champions = vec![];
    let mut current_insiders = vec![];
    let mut current_gms = vec![];
    {
        let guild = discord_api
            .cache
            .guild(crate::discord::sync::ids::GUILD_ID)
            .ok_or_else(|| simple_error::SimpleError::new("Did not find guild in cache"))?;
        for member in &guild.members {
            let user_id = member.user.id;
            let is_champion = member.roles.contains(&ids::CHAMPION_ID);
            let is_gm_champion = member.roles.contains(&ids::GM_CHAMPION_ID);
            let is_insider = member.roles.contains(&ids::INSIDER_ID);
            let is_gm = member
                .roles
                .contains(&crate::discord::sync::ids::GAME_MASTER_ID);
            if is_champion {
                current_champions.push(user_id);
            }
            if is_gm_champion {
                current_gm_champions.push(user_id);
            }
            if is_insider {
                current_insiders.push(user_id);
            }
            if is_gm {
                current_gms.push(user_id);
            }
        }
    }
    // Assign the role(s) to users which earned it but don't have it yet and
    // remove it from those who currently have it, but are not subscribed
    // anymore
    let new_champions: HashSet<_> = new_champions.into_iter().collect();
    let new_insiders: HashSet<_> = new_insiders.into_iter().collect();
    let current_champions: HashSet<_> = current_champions.into_iter().collect();
    let current_insiders: HashSet<_> = current_insiders.into_iter().collect();
    for new_champion in &new_champions {
        if current_gms.contains(new_champion) {
            // Is also a GM
            if !current_gm_champions.contains(new_champion) {
                // Assign GM champion role
                if let Err(err) = add_member_role(
                    discord_api,
                    *new_champion,
                    ids::GM_CHAMPION_ID,
                    Some("Automatic role assignment due to being a GM champion"),
                )
                .await
                {
                    eprintln!("Error in update_roles add_member_role:\n{:#?}", err);
                }
            }
            if current_champions.contains(new_champion) {
                // Remove (non-GM) champion role
                if let Err(err) = remove_member_role(
                    discord_api,
                    *new_champion,
                    ids::CHAMPION_ID,
                    Some("Automatic role removal due to being upgraded to a GM champion"),
                )
                .await
                {
                    eprintln!("Error in update_roles remove_member_role:\n{:#?}", err);
                }
            }
        } else {
            // Is not also a GM
            if !current_champions.contains(new_champion) {
                // Assign champion role
                if let Err(err) = add_member_role(
                    discord_api,
                    *new_champion,
                    ids::CHAMPION_ID,
                    Some("Automatic role assignment due to being a champion"),
                )
                .await
                {
                    eprintln!("Error in update_roles add_member_role:\n{:#?}", err);
                }
            }
            if current_gm_champions.contains(new_champion) {
                // Remove GM champion role
                if let Err(err) = remove_member_role(
                    discord_api,
                    *new_champion,
                    ids::GM_CHAMPION_ID,
                    Some("Automatic role removal due to no longer being a GM champion"),
                )
                .await
                {
                    eprintln!("Error in update_roles remove_member_role:\n{:#?}", err);
                }
            }
        }
    }
    for new_insider in &new_insiders {
        if !current_insiders.contains(new_insider) {
            // Assign insider role
            if let Err(err) = add_member_role(
                discord_api,
                *new_insider,
                ids::INSIDER_ID,
                Some("Automatic role assignment due to being an insider"),
            )
            .await
            {
                eprintln!("Error in update_roles add_member_role:\n{:#?}", err);
            }
        }
    }
    for current_champion in &current_champions {
        if !new_champions.contains(current_champion) {
            // Remove champion role
            if let Err(err) = remove_member_role(
                discord_api,
                *current_champion,
                ids::CHAMPION_ID,
                Some("Automatic role removal due to no longer being a champion"),
            )
            .await
            {
                eprintln!("Error in update_roles remove_member_role:\n{:#?}", err);
            }
        }
    }
    for current_gm_champion in &current_gm_champions {
        if !new_champions.contains(current_gm_champion)
            || !current_gms.contains(current_gm_champion)
        {
            // Remove GM champion role
            if let Err(err) = remove_member_role(
                discord_api,
                *current_gm_champion,
                ids::GM_CHAMPION_ID,
                Some("Automatic role removal due to no longer being a GM or a champion"),
            )
            .await
            {
                eprintln!("Error in update_roles remove_member_role:\n{:#?}", err);
            }
        }
    }
    for current_insider in &current_insiders {
        if !new_insiders.contains(current_insider) {
            // Remove insider role
            if let Err(err) = remove_member_role(
                discord_api,
                *current_insider,
                ids::INSIDER_ID,
                Some("Automatic role removal due to no longer being an insider"),
            )
            .await
            {
                eprintln!("Error in update_roles remove_member_role:\n{:#?}", err);
            }
        }
    }
    Ok(())
}

pub async fn get_customer_and_product(
    client: &stripe::Client,
    subscription: &stripe::Subscription,
) -> Result<(stripe::Customer, stripe::Product), crate::meetup::Error> {
    // First, figure out which product was bought
    // Subscription -> Item -> Price -> Product
    let product = subscription
        .items
        .data
        .first()
        .and_then(|item| item.price.as_ref())
        .and_then(|price| price.product.as_ref())
        .ok_or_else(|| {
            simple_error::SimpleError::new(format!(
                "Could not find a product associated with Stripe subscription {}",
                subscription.id
            ))
        })?;
    let product = match product {
        stripe::Expandable::Object(product) => *product.clone(),
        stripe::Expandable::Id(product_id) => {
            stripe::Product::retrieve(client, &product_id, &[]).await?
        }
    };
    // Now, figure out who the customer is
    let customer = match &subscription.customer {
        stripe::Expandable::Object(customer) => *customer.clone(),
        stripe::Expandable::Id(customer_id) => {
            let customer = stripe::Customer::retrieve(client, customer_id, &[]).await?;
            customer
        }
    };
    Ok((customer, product))
}

async fn ensure_customer_has_discord_id(
    customer: &stripe::Customer,
    client: &stripe::Client,
    discord_api: &crate::discord::CacheAndHttp,
) -> Result<Option<UserId>, crate::meetup::Error> {
    let discord_id = customer
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("_hyperion_discord_id"))
        .map(|id| id.parse::<u64>())
        .transpose()
        .unwrap_or(None);
    if let Some(discord_id) = discord_id {
        Ok(Some(UserId::new(discord_id)))
    } else {
        // No Discord ID is stored in the Stripe metadata.
        // Check for a Discord username, use that to look up the ID and store
        // it in the Stripe metadata.
        let discord_username = match customer
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("Discord"))
        {
            None => return Ok(None),
            Some(username) => username,
        };
        let discord_id = match discord_username_to_id(discord_api, discord_username).await? {
            Some(id) => id,
            None => {
                eprintln!(
                    "Could not find Discord ID for username `{}`",
                    discord_username
                );
                return Ok(None);
            }
        };
        // Try to store the Discord ID in Stripe.
        // Don't fail this method if it doesn't work, just log it.
        let mut new_metadata = HashMap::new();
        new_metadata.insert(
            "_hyperion_discord_id".to_string(),
            format!("{}", discord_id),
        );
        if let Err(err) = stripe::Customer::update(
            client,
            &customer.id,
            stripe::UpdateCustomer {
                metadata: Some(new_metadata),
                ..Default::default()
            },
        )
        .await
        {
            eprintln!(
                "Could not store the Discord user ID in Stripe customer metadata.\nStripe \
                 customer: {}\nError:\n{:#?}",
                customer.id, err
            );
        };
        Ok(Some(discord_id))
    }
}

// TODO: move to discord utils
pub async fn discord_username_to_id(
    discord_api: &crate::discord::CacheAndHttp,
    username: &str,
) -> Result<Option<UserId>, crate::meetup::Error> {
    let mut members = crate::discord::sync::ids::GUILD_ID
        .members_iter(discord_api.http())
        .boxed();
    while let Some(member_result) = members.next().await {
        let member = member_result?;
        if member.user.name == username
            || &format!(
                "{}#{:04}",
                member.user.name,
                member.user.discriminator.map(|d| d.get()).unwrap_or(0)
            ) == username
        {
            return Ok(Some(member.user.id));
        }
    }
    eprintln!("Could not find a Discord ID for username {}", username);
    Ok(None)
}

// TODO: move to discord utils
pub async fn add_member_role(
    discord_api: &crate::discord::CacheAndHttp,
    user_id: UserId,
    role_id: RoleId,
    audit_log_reason: Option<&str>,
) -> Result<(), crate::meetup::Error> {
    match discord_api
        .http()
        .add_member_role(
            crate::discord::sync::ids::GUILD_ID,
            user_id,
            role_id,
            audit_log_reason,
        )
        .await
    {
        Ok(_) => {
            println!("Assigned user {} to role {}", user_id, role_id);
            Ok(())
        }
        Err(err) => {
            eprintln!(
                "Could not assign user {} to role {}:\n{:#?}",
                user_id, role_id, err
            );
            Err(err.into())
        }
    }
}

// TODO: move to discord utils
async fn remove_member_role(
    discord_api: &crate::discord::CacheAndHttp,
    user_id: UserId,
    role_id: RoleId,
    audit_log_reason: Option<&str>,
) -> Result<(), crate::meetup::Error> {
    match discord_api
        .http()
        .remove_member_role(
            crate::discord::sync::ids::GUILD_ID,
            user_id,
            role_id,
            audit_log_reason,
        )
        .await
    {
        Ok(_) => {
            println!("Removed role {} from user {}", role_id, user_id);
            Ok(())
        }
        Err(err) => {
            eprintln!(
                "Could not remove role {} from user {}:\n{:#?}",
                role_id, user_id, err
            );
            Err(err.into())
        }
    }
}
