use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct CommonEventDetails {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub date_time: DateTime<Utc>,
    pub venue: Option<CommonVenue>,
    pub is_online: bool,
    pub num_free_spots: u32,
    pub rsvps_closed: bool,
    pub short_url: String,
}

#[derive(Debug, Clone)]
pub struct CommonVenue {
    pub lat: f64,
    pub lng: f64,
    pub city: Option<String>,
}

impl CommonEventDetails {
    pub fn num_free_spots(&self) -> u32 {
        self.num_free_spots
    }
}

impl From<crate::meetup::newapi::UpcomingEventDetails> for CommonEventDetails {
    fn from(meetup_event: crate::meetup::newapi::UpcomingEventDetails) -> Self {
        let rsvps_closed = meetup_event
            .rsvp_settings
            .as_ref()
            .and_then(|rsvp_settings| rsvp_settings.rsvps_closed)
            .unwrap_or(false);

        let num_free_spots = meetup_event.num_free_spots();

        let venue = meetup_event.venue.map(|v| CommonVenue {
            lat: v.lat,
            lng: v.lng,
            city: v.city,
        });

        CommonEventDetails {
            id: meetup_event.id.0,
            title: meetup_event.title.unwrap_or_else(|| "No title".to_string()),
            description: meetup_event.description,
            date_time: meetup_event.date_time.0,
            venue,
            is_online: meetup_event.is_online,
            num_free_spots,
            rsvps_closed,
            short_url: meetup_event.short_url,
        }
    }
}

impl
    From<(
        &crate::swissrpg::schema::Event,
        &crate::swissrpg::schema::Session,
    )> for CommonEventDetails
{
    fn from(
        (event_series, session): (
            &crate::swissrpg::schema::Event,
            &crate::swissrpg::schema::Session,
        ),
    ) -> Self {
        // For SwissRPG events, we need to determine location from tags
        let venue = event_series
            .tags
            .iter()
            .find(|tag| tag.tag_type == "location")
            .and_then(|location_tag| {
                // Try to match location names to coordinates
                // This is a simplified implementation - in production you might want
                // to store coordinates in the location tags or have a lookup table
                match location_tag.code.as_str() {
                    "zurich" => Some(CommonVenue {
                        lat: 47.376888,
                        lng: 8.541694,
                        city: Some("Zürich".to_string()),
                    }),
                    "basel" => Some(CommonVenue {
                        lat: 47.559601,
                        lng: 7.588576,
                        city: Some("Basel".to_string()),
                    }),
                    "geneva" => Some(CommonVenue {
                        lat: 46.204391,
                        lng: 6.143158,
                        city: Some("Geneva".to_string()),
                    }),
                    "lausanne" => Some(CommonVenue {
                        lat: 46.519316,
                        lng: 6.6345432,
                        city: Some("Lausanne".to_string()),
                    }),
                    "bern" => Some(CommonVenue {
                        lat: 46.9489217,
                        lng: 7.4433158,
                        city: Some("Bern".to_string()),
                    }),
                    "luzern" => Some(CommonVenue {
                        lat: 47.045540,
                        lng: 8.308010,
                        city: Some("Luzern".to_string()),
                    }),
                    "lugano" => Some(CommonVenue {
                        lat: 46.003601,
                        lng: 8.953620,
                        city: Some("Lugano".to_string()),
                    }),
                    "aarau" => Some(CommonVenue {
                        lat: 47.3934732,
                        lng: 8.0606556,
                        city: Some("Aarau".to_string()),
                    }),
                    "chur" => Some(CommonVenue {
                        lat: 46.8533507,
                        lng: 9.5275838,
                        city: Some("Chur".to_string()),
                    }),
                    "st_gallen" => Some(CommonVenue {
                        lat: 47.4256037,
                        lng: 9.3741491,
                        city: Some("St. Gallen".to_string()),
                    }),
                    location_code => {
                        if location_code == "online" {
                            None
                        } else {
                            Some(CommonVenue {
                                lat: 0.0,
                                lng: 0.0,
                                city: Some(location_tag.value.clone()),
                            })
                        }
                    }
                }
            });

        // Check if event is online based on location tag
        let is_online = event_series
            .tags
            .iter()
            .any(|tag| tag.tag_type == "location" && tag.code == "online")
            || venue.is_none();

        CommonEventDetails {
            id: session.uuid.to_string(),
            title: event_series.title.clone(),
            description: event_series.description.clone(),
            date_time: session.start,
            venue,
            is_online,
            num_free_spots: session.open_seats.max(0) as u32,
            rsvps_closed: !session.rsvp_open,
            short_url: event_series.public_url.clone(),
        }
    }
}
