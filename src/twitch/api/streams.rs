use chrono::{DateTime, Utc};
use color_eyre::Result;
use reqwest::Client;
use serde::Deserialize;

use super::{ResponseList, TWITCH_API_BASE_URL};

#[derive(Deserialize, Debug, Clone)]
pub struct StreamResponse {
    started_at: DateTime<Utc>,
}

/// When the broadcaster's current stream started, or `None` if they are offline.
///
/// <https://dev.twitch.tv/docs/api/reference/#get-streams>
pub async fn get_stream_started_at(
    client: &Client,
    broadcaster_id: &str,
) -> Result<Option<DateTime<Utc>>> {
    let url = format!("{TWITCH_API_BASE_URL}/streams?user_id={broadcaster_id}");

    let response_data = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json::<ResponseList<StreamResponse>>()
        .await?;

    Ok(response_data.data.first().map(|stream| stream.started_at))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::StreamResponse;
    use crate::twitch::api::ResponseList;

    #[test]
    fn live_stream_has_started_at() {
        let body = r#"{"data":[{"id":"1","user_id":"1111","user_login":"someone","type":"live",
            "title":"t","viewer_count":5,"started_at":"2026-11-06T01:02:03Z","tags":[]}],
            "pagination":{}}"#;
        let list: ResponseList<StreamResponse> = serde_json::from_str(body).unwrap();

        assert_eq!(
            list.data.first().map(|s| s.started_at),
            Some(Utc.with_ymd_and_hms(2026, 11, 6, 1, 2, 3).unwrap())
        );
    }

    #[test]
    fn offline_stream_has_no_data() {
        let list: ResponseList<StreamResponse> =
            serde_json::from_str(r#"{"data":[],"pagination":{}}"#).unwrap();

        assert!(list.data.is_empty());
    }
}
