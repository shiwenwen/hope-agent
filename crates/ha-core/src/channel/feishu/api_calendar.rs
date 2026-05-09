//! calendar (日历 / Lark Calendar) REST methods.
//!
//! Six endpoints:
//! - `calendar_list` — list calendars the user has access to
//! - `calendar_create_event` — create a new event in a calendar
//! - `calendar_list_events` — list events with optional time range
//! - `calendar_update_event` — patch an event
//! - `calendar_delete_event` — delete an event
//! - `calendar_attendees_create` — invite attendees to an event
//!
//! Reference: <https://open.feishu.cn/document/server-docs/calendar-v4/calendar/list>

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::FeishuApi;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CalendarListResult {
    #[serde(default)]
    pub calendar_list: Vec<Value>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CalendarEventResult {
    /// Pass-through of Feishu's `event` object — full event JSON.
    #[serde(default)]
    pub event: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CalendarEventsList {
    #[serde(default)]
    pub items: Vec<Value>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CalendarAttendeesResult {
    #[serde(default)]
    pub attendees: Vec<Value>,
}

impl FeishuApi {
    /// `GET /open-apis/calendar/v4/calendars` — list calendars accessible
    /// to the bot's tenant.
    pub async fn calendar_list(
        &self,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<CalendarListResult> {
        let mut url = format!("{}/open-apis/calendar/v4/calendars", self.base_url());
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(t) = page_token {
            params.push(("page_token", t.to_string()));
        }
        if let Some(s) = page_size {
            params.push(("page_size", s.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET calendar_list: {}", e))?;
        Ok(self
            .parse_envelope::<CalendarListResult>(resp, "calendar_list")
            .await?
            .unwrap_or_default())
    }

    /// `POST /open-apis/calendar/v4/calendars/{calendar_id}/events` — create event.
    pub async fn calendar_create_event(
        &self,
        calendar_id: &str,
        event: Value,
    ) -> Result<CalendarEventResult> {
        let url = format!(
            "{}/open-apis/calendar/v4/calendars/{}/events",
            self.base_url(),
            calendar_id
        );
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&event)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST calendar_create_event: {}", e))?;
        Ok(self
            .parse_envelope::<CalendarEventResult>(resp, "calendar_create_event")
            .await?
            .unwrap_or_default())
    }

    /// `GET /open-apis/calendar/v4/calendars/{calendar_id}/events` — list events.
    /// `start_time` / `end_time` are RFC3339 strings (or epoch-second strings).
    pub async fn calendar_list_events(
        &self,
        calendar_id: &str,
        start_time: Option<&str>,
        end_time: Option<&str>,
        page_token: Option<&str>,
        page_size: Option<u32>,
    ) -> Result<CalendarEventsList> {
        let mut url = format!(
            "{}/open-apis/calendar/v4/calendars/{}/events",
            self.base_url(),
            calendar_id
        );
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(v) = start_time {
            params.push(("start_time", v.to_string()));
        }
        if let Some(v) = end_time {
            params.push(("end_time", v.to_string()));
        }
        if let Some(v) = page_token {
            params.push(("page_token", v.to_string()));
        }
        if let Some(v) = page_size {
            params.push(("page_size", v.to_string()));
        }
        super::api::append_query(&mut url, &params);
        let resp = self
            .authorized_request(reqwest::Method::GET, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to GET calendar_list_events: {}", e))?;
        Ok(self
            .parse_envelope::<CalendarEventsList>(resp, "calendar_list_events")
            .await?
            .unwrap_or_default())
    }

    /// `PATCH /open-apis/calendar/v4/calendars/{calendar_id}/events/{event_id}` — patch event.
    pub async fn calendar_update_event(
        &self,
        calendar_id: &str,
        event_id: &str,
        patch: Value,
    ) -> Result<CalendarEventResult> {
        let url = format!(
            "{}/open-apis/calendar/v4/calendars/{}/events/{}",
            self.base_url(),
            calendar_id,
            event_id
        );
        let resp = self
            .authorized_request(reqwest::Method::PATCH, &url)
            .await?
            .json(&patch)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to PATCH calendar_update_event: {}", e))?;
        Ok(self
            .parse_envelope::<CalendarEventResult>(resp, "calendar_update_event")
            .await?
            .unwrap_or_default())
    }

    /// `DELETE /open-apis/calendar/v4/calendars/{calendar_id}/events/{event_id}`
    pub async fn calendar_delete_event(&self, calendar_id: &str, event_id: &str) -> Result<()> {
        let url = format!(
            "{}/open-apis/calendar/v4/calendars/{}/events/{}",
            self.base_url(),
            calendar_id,
            event_id
        );
        let resp = self
            .authorized_request(reqwest::Method::DELETE, &url)
            .await?
            .send()
            .await
            .map_err(|e| anyhow!("Failed to DELETE calendar_delete_event: {}", e))?;
        let _: Option<Value> = self.parse_envelope(resp, "calendar_delete_event").await?;
        Ok(())
    }

    /// `POST /open-apis/calendar/v4/calendars/{calendar_id}/events/{event_id}/attendees`
    /// — invite attendees. `attendees` is the array per Feishu's schema
    /// (each entry typically `{type, user_id|chat_id|third_party_email, ...}`).
    pub async fn calendar_attendees_create(
        &self,
        calendar_id: &str,
        event_id: &str,
        attendees: Value,
    ) -> Result<CalendarAttendeesResult> {
        let url = format!(
            "{}/open-apis/calendar/v4/calendars/{}/events/{}/attendees",
            self.base_url(),
            calendar_id,
            event_id
        );
        let body = serde_json::json!({"attendees": attendees});
        let resp = self
            .authorized_request(reqwest::Method::POST, &url)
            .await?
            .json(&body)
            .send()
            .await
            .map_err(|e| anyhow!("Failed to POST calendar_attendees_create: {}", e))?;
        Ok(self
            .parse_envelope::<CalendarAttendeesResult>(resp, "calendar_attendees_create")
            .await?
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    use super::super::api::test_support::mock_api;

    #[tokio::test]
    async fn list_returns_calendars() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("GET"))
            .and(path("/open-apis/calendar/v4/calendars"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"calendar_list": [{"calendar_id": "cal1"}], "has_more": false}
            })))
            .mount(&server)
            .await;
        let r = api.calendar_list(None, None).await.unwrap();
        assert_eq!(r.calendar_list.len(), 1);
    }

    #[tokio::test]
    async fn create_event_passes_body_through() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("POST"))
            .and(path("/open-apis/calendar/v4/calendars/cal1/events"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"event": {"event_id": "ev1"}}
            })))
            .mount(&server)
            .await;
        let r = api
            .calendar_create_event("cal1", serde_json::json!({"summary": "team sync"}))
            .await
            .unwrap();
        assert_eq!(r.event["event_id"], "ev1");
    }

    #[tokio::test]
    async fn delete_event_returns_ok() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("DELETE"))
            .and(path("/open-apis/calendar/v4/calendars/cal1/events/ev1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok"
            })))
            .mount(&server)
            .await;
        api.calendar_delete_event("cal1", "ev1").await.unwrap();
    }

    #[tokio::test]
    async fn attendees_create_wraps_array_in_body() {
        let server = MockServer::start().await;
        let api = mock_api(&server).await;
        Mock::given(method("POST"))
            .and(path(
                "/open-apis/calendar/v4/calendars/cal1/events/ev1/attendees",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "code": 0, "msg": "ok",
                "data": {"attendees": [{"type": "user"}]}
            })))
            .mount(&server)
            .await;
        let r = api
            .calendar_attendees_create("cal1", "ev1", serde_json::json!([{"type": "user"}]))
            .await
            .unwrap();
        assert_eq!(r.attendees.len(), 1);
    }
}
