//! The HTTP half of the Google Calendar integration.
//!
//! Deliberately thin. Everything that decides anything — what an event means,
//! which copy of a duplicated invitation wins, which recording belongs to
//! which meeting — lives in [`super::parse`] and [`super::agenda`], where it is
//! tested without a network. What is left here is building three URLs,
//! refreshing a token when it has expired, and paging until Google stops
//! sending a page token.

use chrono::Utc;

use crate::oauth::{refresh_google_access_token, OAuthTokens};

use super::model::CalendarEvent;
use super::parse;

const CALENDAR_LIST_URL: &str = "https://www.googleapis.com/calendar/v3/users/me/calendarList";
const EVENTS_URL_BASE: &str = "https://www.googleapis.com/calendar/v3/calendars";

/// Pages fetched for one calendar before the sync gives up.
///
/// A guard against a paging bug turning into an unbounded loop against
/// Google's API, not a limit anyone should reach: 2500 events is several years
/// of a busy calendar and the sync window is measured in weeks.
const MAX_PAGES: usize = 10;

/// Events per page. Google's own maximum is 2500; this is smaller so a slow
/// link shows progress rather than stalling on one enormous response.
const PAGE_SIZE: u32 = 250;

/// Seconds before expiry at which a token is treated as already expired.
///
/// A token that expires during the request it authorises fails the request.
const REFRESH_MARGIN_SECONDS: i64 = 120;

#[derive(Debug, thiserror::Error)]
pub enum CalendarApiError {
    #[error("the calendar could not be reached: {0}")]
    Network(String),

    #[error("Google refused the request: {0}")]
    Refused(String),

    #[error("this account needs to be reconnected: {0}")]
    Reauthorize(String),
}

/// A calendar the account can read.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CalendarSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub selected: bool,
}

/// Returns a usable access token, refreshing first when the stored one has
/// expired.
///
/// Returns the refreshed tokens too, so the caller can persist them: a refresh
/// whose result is thrown away means refreshing again on the next sync, and on
/// every sync after that.
pub async fn fresh_access_token(
    tokens: &OAuthTokens,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<(String, Option<OAuthTokens>), CalendarApiError> {
    if tokens.expires_at - REFRESH_MARGIN_SECONDS > Utc::now().timestamp() {
        return Ok((tokens.access_token.clone(), None));
    }
    let Some(refresh_token) = tokens.refresh_token.as_deref() else {
        // Google issues a refresh token only on the first consent for an
        // account. Without one there is nothing to refresh and the only fix is
        // signing in again, so say that rather than reporting a network error.
        return Err(CalendarApiError::Reauthorize(
            "no refresh token is stored for this account".to_string(),
        ));
    };

    let mut refreshed = refresh_google_access_token(client_id, client_secret, refresh_token)
        .await
        .map_err(CalendarApiError::Reauthorize)?;
    // A refresh response does not repeat the refresh token or the account it
    // belongs to; carrying them forward is what keeps the stored record whole.
    if refreshed.refresh_token.is_none() {
        refreshed.refresh_token = tokens.refresh_token.clone();
    }
    if refreshed.account_email.is_none() {
        refreshed.account_email = tokens.account_email.clone();
    }
    Ok((refreshed.access_token.clone(), Some(refreshed)))
}

/// The calendars this account can read.
pub async fn list_calendars(access_token: &str) -> Result<Vec<CalendarSummary>, CalendarApiError> {
    let body = get_json(CALENDAR_LIST_URL, access_token, &[]).await?;
    Ok(body
        .get("items")
        .and_then(|items| items.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some(CalendarSummary {
                        id: item.get("id")?.as_str()?.to_string(),
                        name: item
                            .get("summary")
                            .and_then(|s| s.as_str())
                            .unwrap_or("Calendar")
                            .to_string(),
                        primary: item.get("primary").and_then(|p| p.as_bool()) == Some(true),
                        selected: item.get("selected").and_then(|s| s.as_bool()) != Some(false),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// Every event in `[time_min, time_max]` on one calendar.
///
/// `singleEvents=true` is what turns a recurring rule into the individual
/// occurrences a person actually attends; without it a weekly standup is one
/// event in January and the agenda for today is empty.
pub async fn list_events(
    access_token: &str,
    account_email: &str,
    calendar_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<CalendarEvent>, CalendarApiError> {
    let url = format!(
        "{EVENTS_URL_BASE}/{}/events",
        urlencoding::encode(calendar_id)
    );
    let page_size = PAGE_SIZE.to_string();

    let mut events = Vec::new();
    let mut page_token: Option<String> = None;

    for _ in 0..MAX_PAGES {
        let mut query: Vec<(&str, &str)> = vec![
            ("timeMin", time_min),
            ("timeMax", time_max),
            ("singleEvents", "true"),
            ("orderBy", "startTime"),
            ("maxResults", page_size.as_str()),
        ];
        if let Some(token) = page_token.as_deref() {
            query.push(("pageToken", token));
        }

        let body = get_json(&url, access_token, &query).await?;
        events.extend(parse::parse_events(&body, account_email, calendar_id));

        page_token = parse::next_page_token(&body);
        if page_token.is_none() {
            break;
        }
    }
    Ok(events)
}

async fn get_json(
    url: &str,
    access_token: &str,
    query: &[(&str, &str)],
) -> Result<serde_json::Value, CalendarApiError> {
    let response = reqwest::Client::new()
        .get(url)
        .bearer_auth(access_token)
        .query(query)
        .send()
        .await
        .map_err(|err| CalendarApiError::Network(err.to_string()))?;

    let status = response.status();
    if status.is_success() {
        return response
            .json()
            .await
            .map_err(|err| CalendarApiError::Network(err.to_string()));
    }

    let detail = response.text().await.unwrap_or_default();
    // 401 and 403 are the two a user can actually do something about, and what
    // they do about them is sign in again. Everything else is Google's problem
    // and will pass.
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        Err(CalendarApiError::Reauthorize(summarise(&detail)))
    } else {
        Err(CalendarApiError::Refused(format!(
            "{} {}",
            status.as_u16(),
            summarise(&detail)
        )))
    }
}

/// The readable part of a Google error body.
///
/// Google's errors are a JSON envelope around one useful sentence. Showing the
/// envelope teaches the user nothing; showing nothing teaches them less.
fn summarise(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| {
            let trimmed = body.trim();
            if trimmed.is_empty() {
                "no detail was returned".to_string()
            } else {
                trimmed.chars().take(200).collect()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(expires_at: i64, refresh: Option<&str>) -> OAuthTokens {
        OAuthTokens {
            access_token: "stored-token".into(),
            refresh_token: refresh.map(str::to_string),
            token_type: "Bearer".into(),
            expires_at,
            scope: None,
            account_email: Some("me@work.com".into()),
            account_name: None,
            last_synced_at: None,
        }
    }

    #[tokio::test]
    async fn a_token_with_time_left_is_used_as_is() {
        let valid = tokens(Utc::now().timestamp() + 3_600, Some("refresh"));
        let (token, refreshed) = fresh_access_token(&valid, None, None)
            .await
            .expect("the stored token");
        assert_eq!(token, "stored-token");
        assert!(refreshed.is_none(), "nothing to persist when nothing changed");
    }

    #[tokio::test]
    async fn a_token_about_to_expire_is_not_treated_as_valid() {
        // A token that expires during the request it authorises fails the
        // request, so the margin has to bite before the expiry does.
        let expiring = tokens(Utc::now().timestamp() + 30, None);
        let result = fresh_access_token(&expiring, None, None).await;
        assert!(matches!(result, Err(CalendarApiError::Reauthorize(_))));
    }

    #[tokio::test]
    async fn an_account_with_no_refresh_token_is_told_to_reconnect() {
        // Google issues one only on first consent. There is nothing to refresh
        // and nothing a retry would fix.
        let expired = tokens(0, None);
        match fresh_access_token(&expired, None, None).await {
            Err(CalendarApiError::Reauthorize(reason)) => {
                assert!(reason.contains("refresh token"), "got {reason}");
            }
            other => panic!("expected a reconnect prompt, got {other:?}"),
        }
    }

    #[test]
    fn a_google_error_is_reduced_to_the_sentence_inside_it() {
        let body = r#"{"error":{"code":403,"message":"Request had insufficient authentication scopes.","errors":[]}}"#;
        assert_eq!(
            summarise(body),
            "Request had insufficient authentication scopes."
        );
    }

    #[test]
    fn an_error_body_that_is_not_json_still_says_something() {
        assert_eq!(summarise("<html>502 Bad Gateway</html>"), "<html>502 Bad Gateway</html>");
        assert_eq!(summarise("   "), "no detail was returned");
    }

    #[test]
    fn an_enormous_error_body_is_cut_to_something_showable() {
        let huge = "x".repeat(5_000);
        assert_eq!(summarise(&huge).chars().count(), 200);
    }

    #[test]
    fn a_calendar_id_with_an_at_sign_survives_being_put_in_a_url() {
        // Every Google calendar id is an address, so this is the normal case
        // rather than an edge one.
        let encoded = urlencoding::encode("me@work.com");
        assert_eq!(encoded, "me%40work.com");
    }
}
