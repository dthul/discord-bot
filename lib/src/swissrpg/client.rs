use super::schema::{
    Event, Location, LocationsResponse, MigrateEventRequest, ScheduleSessionRequest, Tag,
};
use reqwest::Client;
use simple_error::SimpleError;
use uuid;

pub struct SwissRPGClient {
    client: Client,
    base_url: String,
    auth_token: String,
}

impl SwissRPGClient {
    pub fn new(base_url: String, auth_token: String) -> Self {
        Self {
            client: Client::new(),
            base_url,
            auth_token,
        }
    }

    pub async fn get_events(&self) -> Result<Vec<Event>, crate::BoxedError> {
        let url = format!("{}/api/events", self.base_url);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.auth_token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(SimpleError::new(format!(
                "API request failed with status: {}",
                response.status()
            ))
            .into());
        }

        let events: Vec<Event> = response.json().await?;
        Ok(events)
    }

    pub async fn get_locations(&self) -> Result<Vec<Location>, crate::BoxedError> {
        let url = format!("{}/api/tags/location", self.base_url);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.auth_token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(SimpleError::new(format!(
                "API request failed with status: {}",
                response.status()
            ))
            .into());
        }

        let locations_response: LocationsResponse = response.json().await?;
        Ok(locations_response.locations)
    }

    pub async fn get_tags_by_type(&self, tag_type: &str) -> Result<Vec<Tag>, crate::BoxedError> {
        let url = format!("{}/api/tags/{}", self.base_url, tag_type);
        let response = self
            .client
            .get(&url)
            .bearer_auth(&self.auth_token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(SimpleError::new(format!(
                "API request failed with status: {}",
                response.status()
            ))
            .into());
        }

        let tags: Vec<Tag> = response.json().await?;
        Ok(tags)
    }

    #[tracing::instrument(skip(self, request), fields(event_uuid = %event_uuid))]
    pub async fn schedule_session(
        &self,
        event_uuid: &uuid::Uuid,
        request: ScheduleSessionRequest,
    ) -> Result<Event, crate::BoxedError> {
        let url = format!("{}/api/events/{}", self.base_url, event_uuid);
        let response = self
            .client
            .put(&url)
            .bearer_auth(&self.auth_token)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(SimpleError::new(format!(
                "API request failed with status: {}",
                response.status()
            ))
            .into());
        }

        let event: Event = response.json().await?;
        Ok(event)
    }

    pub async fn delete_event(&self, event_uuid: &uuid::Uuid) -> Result<(), crate::BoxedError> {
        let url = format!("{}/api/events/{}", self.base_url, event_uuid);
        let response = self
            .client
            .delete(&url)
            .bearer_auth(&self.auth_token)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(SimpleError::new(format!(
                "API request failed with status: {}",
                response.status()
            ))
            .into());
        }

        Ok(())
    }

    pub async fn migrate_event(
        &self,
        request: MigrateEventRequest,
    ) -> Result<Event, crate::BoxedError> {
        let url = format!("{}/api/migrate", self.base_url);
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.auth_token)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await;
            println!("Migrate request failed:\n{} {:#?}", status, text);
            return Err(
                SimpleError::new(format!("API request failed with status: {}", status)).into(),
            );
        }

        let event: Event = response.json().await?;
        Ok(event)
    }
}
