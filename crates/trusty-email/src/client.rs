//! Gmail REST API client.

use anyhow::Result;
use trusty_models::email::EmailMessage;

/// Wraps the Gmail API with typed methods.
#[allow(dead_code)]
pub struct GmailClient {
    http: reqwest::Client,
    access_token: String,
}

impl GmailClient {
    /// Construct from a valid access token.
    pub fn new(access_token: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { http, access_token }
    }

    /// Fetch the list of SENT message IDs modified since `history_id`.
    ///
    /// Uses the Gmail History API for incremental polling.
    pub async fn list_history_since(&self, _history_id: &str) -> Result<Vec<String>> {
        todo!("GET /gmail/v1/users/me/history?historyTypes=messageAdded&labelId=SENT")
    }

    /// Fetch and decode a single message by ID.
    pub async fn get_message(&self, _message_id: &str) -> Result<EmailMessage> {
        todo!("GET /gmail/v1/users/me/messages/{{id}}?format=full, decode MIME parts and headers")
    }

    /// Return the authenticated user's email address and display name.
    pub async fn get_user_profile(&self) -> Result<(String, String)> {
        todo!("GET /gmail/v1/users/me/profile")
    }

    /// Return the current Gmail `historyId` for the authenticated mailbox.
    pub async fn get_history_id(&self) -> Result<String> {
        todo!("GET /gmail/v1/users/me/profile and return historyId field")
    }
}
