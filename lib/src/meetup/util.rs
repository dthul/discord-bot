use unicode_segmentation::UnicodeSegmentation;

use crate::meetup::newapi::IsOnline;

pub async fn clone_event<'a>(
    urlname: &'a str,
    event_id: &'a str,
    meetup_client: &'a super::newapi::AsyncClient,
    hook: Option<
        Box<
            dyn FnOnce(super::newapi::NewEvent) -> Result<super::newapi::NewEvent, super::Error>
                + Send
                + 'a,
        >,
    >,
) -> Result<super::newapi::NewEventResponse, super::Error> {
    let event = meetup_client.get_event(event_id.into()).await?;
    let new_event = super::newapi::NewEvent {
        group_urlname: urlname.into(),
        title: event.title,
        description: event.description,
        start_date_time: event.date_time.into(),
        duration: None,
        rsvp_settings: Some(super::newapi::NewEventRsvpSettings {
            rsvp_limit: event.max_tickets,
            guest_limit: Some(event.number_of_allowed_guests),
            rsvp_open_time: None,
            rsvp_close_time: None,
            rsvp_open_duration: None,
            rsvp_close_duration: None,
        }),
        event_hosts: Some(
            event
                .event_hosts
                .unwrap_or(vec![])
                .iter()
                .filter_map(|maybe_host| maybe_host.and_then(|host| host.member))
                .filter_map(|member| member.id.parse::<i64>().ok())
                .collect(),
        ),
        venue_ids: event
            .venues
            .map(|venues| venues.iter().map(|venue| venue.id.clone()).collect()),
        self_rsvp: Some(false),
        how_to_find_us: event.how_to_find_us,
        question: None,
        featured_photo_id: Some(event.image.id.0 as i64),
        publish_status: Some(super::newapi::NewEventPublishStatus::DRAFT),
    };
    // If there is a hook specified, let it modify the new event before
    // publishing it to Meetup
    let new_event = match hook {
        Some(hook) => hook(new_event)?,
        None => new_event,
    };
    // Post the event on Meetup
    let new_event = meetup_client.create_event(new_event).await?;
    return Ok(new_event);
}

pub async fn get_group_memberships(
    meetup_api: super::newapi::AsyncClient,
) -> Result<Vec<super::newapi::GroupMembership>, super::newapi::Error> {
    let mut memberships = Vec::with_capacity(super::newapi::URLNAMES.len());
    for urlname in super::newapi::URLNAMES {
        let membership = meetup_api.get_group_membership(urlname.to_string()).await?;
        memberships.push(membership);
    }
    Ok(memberships)
}

// Truncates the string to the given maximum length.
// The "length" of a Unicode string is no well-defined concept. Meetup (at least
// the form on the website) seems to use the number of UTF-16 code units as the
// length of the string, so this is what this method uses (see
// https://hsivonen.fi/string-length/ for some details on Unicode string lengths).
pub fn truncate_str(mut string: String, max_len: usize) -> String {
    // Count the number of characters that are allowed
    let mut utf16_length = 0;
    let mut utf8_length = 0;
    for grapheme in UnicodeSegmentation::graphemes(string.as_str(), /*extended*/ true) {
        // Compute the length of this grapheme in UTF-16
        let grapheme_utf16_length = grapheme.encode_utf16().count();
        if utf16_length + grapheme_utf16_length <= max_len {
            utf16_length += grapheme_utf16_length;
            utf8_length += grapheme.len();
        } else {
            // We have reached the maximum length
            break;
        }
    }
    string.truncate(utf8_length);
    string
}
