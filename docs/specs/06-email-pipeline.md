# Email Pipeline

## Overview

The email pipeline handles two concerns: OAuth2 token lifecycle management and Gmail data ingestion. Both are implemented in `trusty-email`. The pipeline is driven by `trusty-daemon` on a configurable schedule.

Only the SENT folder is processed. This is a deliberate design decision: sent emails reflect the user's relationships, commitments, and communication style. Received emails add noise (mass CC lists, newsletters, automated notifications) and would require significantly more aggressive filtering.

---

## Gmail OAuth2 Flow

### Application Registration

The user must create a Google Cloud Console project and register an OAuth2 application. trusty-izzie uses the "Desktop app" OAuth client type (not web app), which enables the local redirect server flow.

**Required OAuth scopes:**
```
https://www.googleapis.com/auth/gmail.readonly
https://www.googleapis.com/auth/calendar.readonly
```

**Credentials stored in config:**
```toml
[google]
client_id     = "123456789.apps.googleusercontent.com"
client_secret = "GOCSPX-..."
```

### Authorization Flow

The flow is initiated via `POST /api/auth/google` (HTTP API) or `trusty-cli auth google` (CLI).

```
User triggers auth
       │
       ▼
trusty-email: OAuthFlow::start_auth_flow()
       │
       ├── Generate code_verifier + code_challenge (PKCE)
       ├── Build authorization URL:
       │     https://accounts.google.com/o/oauth2/v2/auth
       │       ?client_id={client_id}
       │       &redirect_uri=http://localhost:8080/callback
       │       &response_type=code
       │       &scope=gmail.readonly+calendar.readonly
       │       &access_type=offline
       │       &prompt=consent
       │       &code_challenge={code_challenge}
       │       &code_challenge_method=S256
       │
       ├── Start local HTTP server on port 8080 (tiny_http)
       ├── Open URL in browser (open crate / xdg-open / start)
       │
       ▼
User logs in to Google in browser
       │
       ▼
Google redirects to http://localhost:8080/callback?code={code}&state={state}
       │
       ▼
OAuthFlow::exchange_code(code)
       │
       ├── POST https://oauth2.googleapis.com/token
       │     grant_type=authorization_code
       │     code={code}
       │     redirect_uri=http://localhost:8080/callback
       │     client_id={client_id}
       │     client_secret={client_secret}
       │     code_verifier={code_verifier}
       │
       ├── Receive: access_token, refresh_token, expires_in
       │
       └── Store in SQLite oauth_tokens table
```

**Local redirect server implementation:**

```rust
pub async fn start_auth_flow(
    client_id: &str,
    client_secret: &str,
    config: &GoogleConfig,
) -> TrustyResult<OAuthTokens> {
    let (code_verifier, code_challenge) = generate_pkce_pair();
    let state = generate_random_state();

    let auth_url = build_auth_url(client_id, &code_challenge, &state, config);

    // Open browser
    open::that(&auth_url)
        .map_err(|_| TrustyError::AuthError("Failed to open browser".into()))?;

    // Start local server — blocks until callback received (timeout: 5 min)
    let code = listen_for_callback(8080, &state, Duration::from_secs(300)).await?;

    // Exchange code for tokens
    exchange_code(&code, client_id, client_secret, &code_verifier).await
}

async fn listen_for_callback(
    port: u16,
    expected_state: &str,
    timeout: Duration,
) -> TrustyResult<String> {
    let server = tiny_http::Server::http(format!("0.0.0.0:{port}"))
        .map_err(|e| TrustyError::AuthError(e.to_string()))?;

    let deadline = Instant::now() + timeout;

    loop {
        if Instant::now() > deadline {
            return Err(TrustyError::AuthError("OAuth timeout".into()));
        }

        if let Ok(Some(request)) = server.recv_timeout(Duration::from_millis(100)) {
            let url = request.url().to_string();
            let params = parse_query_string(&url);

            // Validate state to prevent CSRF
            if params.get("state").map(|s| s.as_str()) != Some(expected_state) {
                return Err(TrustyError::AuthError("State mismatch".into()));
            }

            if let Some(error) = params.get("error") {
                return Err(TrustyError::AuthError(format!("OAuth error: {error}")));
            }

            let code = params.get("code")
                .ok_or(TrustyError::AuthError("No code in callback".into()))?
                .clone();

            // Send success page to browser
            let response = tiny_http::Response::from_string(SUCCESS_HTML);
            let _ = request.respond(response);

            return Ok(code);
        }
    }
}
```

### Token Storage and Refresh

Tokens are stored in SQLite's `oauth_tokens` table. Before every Gmail API call, the client checks expiry:

```rust
impl GmailClient {
    pub async fn refresh_token_if_needed(&self) -> TrustyResult<()> {
        let token = self.store.get_token(&self.account_id).await?;

        let expires_at = DateTime::parse_from_rfc3339(&token.expires_at)?;
        let now = Utc::now();

        // Refresh if within 5 minutes of expiry
        if (expires_at - now).num_seconds() < 300 {
            self.refresh_access_token(&token.refresh_token).await?;
        }

        Ok(())
    }

    async fn refresh_access_token(&self, refresh_token: &str) -> TrustyResult<()> {
        let response = self.http.post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type",    "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id",     &self.client_id),
                ("client_secret", &self.client_secret),
            ])
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TrustyError::AuthError(
                format!("Token refresh failed: {}", response.status())
            ));
        }

        let refreshed: RefreshResponse = response.json().await?;
        self.store.update_token(&self.account_id, &refreshed).await?;

        Ok(())
    }
}
```

---

## Gmail Incremental Sync

### History-Based Incremental Sync

Gmail's `history.list` API provides efficient incremental sync. Instead of re-fetching all emails, we track a `historyId` (a monotonically increasing integer) and only fetch changes since that ID.

```rust
pub struct GmailClient {
    http:        reqwest::Client,
    account_id:  String,
    client_id:   String,
    client_secret: String,
    store:       Arc<Store>,
}

pub async fn sync_sent(
    &self,
    cursor: Option<HistoryId>,
    batch_size: u32,
) -> TrustyResult<SyncResult> {
    self.refresh_token_if_needed().await?;
    let token = self.store.get_token(&self.account_id).await?;

    match cursor {
        None => {
            // Initial full sync: list all messages in SENT
            self.initial_full_sync(batch_size, &token.access_token).await
        }
        Some(history_id) => {
            // Incremental sync: use history API
            self.incremental_sync(history_id, batch_size, &token.access_token).await
        }
    }
}

async fn incremental_sync(
    &self,
    history_id: HistoryId,
    batch_size: u32,
    access_token: &str,
) -> TrustyResult<SyncResult> {
    let url = "https://gmail.googleapis.com/gmail/v1/users/me/history";

    let response = self.http.get(url)
        .bearer_auth(access_token)
        .query(&[
            ("startHistoryId", history_id.to_string()),
            ("labelId",        "SENT".to_string()),
            ("historyTypes",   "messageAdded".to_string()),
            ("maxResults",     batch_size.to_string()),
        ])
        .send()
        .await?;

    match response.status() {
        StatusCode::OK => {
            let history: GmailHistoryResponse = response.json().await?;
            let message_ids: Vec<String> = history.history
                .unwrap_or_default()
                .into_iter()
                .flat_map(|h| h.messages_added.unwrap_or_default())
                .map(|m| m.id)
                .collect();

            // Deduplicate message IDs
            let mut unique_ids: Vec<String> = message_ids;
            unique_ids.dedup();

            // Fetch full message details
            let emails = self.fetch_messages_batch(&unique_ids, access_token).await?;

            Ok(SyncResult {
                emails,
                new_cursor: HistoryId(history.history_id),
                has_more: history.next_page_token.is_some(),
                page_token: history.next_page_token,
                rate_limited: false,
            })
        }
        StatusCode::TOO_MANY_REQUESTS => {
            Ok(SyncResult { rate_limited: true, ..Default::default() })
        }
        StatusCode::GONE => {
            // historyId too old — must do full sync
            Err(TrustyError::CursorExpired)
        }
        status => {
            Err(TrustyError::ApiError(format!("Gmail API returned {status}")))
        }
    }
}
```

**Handling `GONE` (410) errors:**

If the `historyId` is too old (Gmail only keeps history for ~30 days), the API returns 410. The daemon handles this by clearing the cursor and triggering a fresh full sync:

```rust
match client.sync_sent(cursor, BATCH_SIZE).await {
    Err(TrustyError::CursorExpired) => {
        tracing::warn!("Gmail history cursor expired, starting full resync");
        store.sessions().clear_cursor(&account_id).await?;
        // Next cycle will do a full sync with cursor=None
    }
    // ...
}
```

### Message Fetching

```rust
async fn fetch_messages_batch(
    &self,
    ids: &[String],
    access_token: &str,
) -> TrustyResult<Vec<RawEmail>> {
    // Fetch up to 10 messages concurrently (respect rate limits)
    let chunks: Vec<&[String]> = ids.chunks(10).collect();
    let mut emails = Vec::new();

    for chunk in chunks {
        let futures: Vec<_> = chunk.iter().map(|id| {
            self.fetch_one_message(id, access_token)
        }).collect();

        let results = futures::future::join_all(futures).await;
        for result in results {
            match result {
                Ok(email) => emails.push(email),
                Err(e) => tracing::warn!("Failed to fetch message: {e}"),
            }
        }
    }

    Ok(emails)
}

async fn fetch_one_message(&self, id: &str, access_token: &str) -> TrustyResult<RawEmail> {
    let url = format!(
        "https://gmail.googleapis.com/gmail/v1/users/me/messages/{id}?format=full"
    );

    let response = self.http.get(&url)
        .bearer_auth(access_token)
        .send()
        .await?;

    let raw: GmailMessage = response.json().await?;
    Ok(Self::parse_gmail_message(raw))
}

fn parse_gmail_message(raw: GmailMessage) -> RawEmail {
    let headers: HashMap<String, String> = raw.payload.headers.iter()
        .map(|h| (h.name.to_lowercase(), h.value.clone()))
        .collect();

    let body = extract_body_text(&raw.payload);
    let body_stripped = strip_quoted_replies(&body);

    RawEmail {
        message_id:  raw.id,
        thread_id:   raw.thread_id,
        from:        headers.get("from").cloned().unwrap_or_default(),
        to:          headers.get("to").cloned().unwrap_or_default(),
        cc:          headers.get("cc").cloned().unwrap_or_default(),
        subject:     headers.get("subject").cloned().unwrap_or_default(),
        date:        headers.get("date").cloned().unwrap_or_default(),
        body:        body_stripped,
        raw_body:    body,
    }
}
```

### Body Text Extraction and Quote Stripping

```rust
fn extract_body_text(payload: &GmailPayload) -> String {
    // Prefer text/plain over text/html
    if payload.mime_type == "text/plain" {
        if let Some(data) = &payload.body.data {
            return decode_base64_url(data);
        }
    }

    // Recurse into multipart
    if let Some(parts) = &payload.parts {
        for part in parts {
            if part.mime_type == "text/plain" {
                if let Some(data) = &part.body.data {
                    return decode_base64_url(data);
                }
            }
        }
        // Fallback: try text/html and strip tags
        for part in parts {
            if part.mime_type == "text/html" {
                if let Some(data) = &part.body.data {
                    let html = decode_base64_url(data);
                    return strip_html_tags(&html);
                }
            }
        }
    }

    String::new()
}

fn strip_quoted_replies(body: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut in_quote = false;

    for line in body.lines() {
        // Gmail-style quote: "On Mon, Jan 1 2024, Alice wrote:"
        if line.trim_start().starts_with("On ") && line.trim_end().ends_with("wrote:") {
            in_quote = true;
            break;
        }
        // Standard email quote: lines starting with ">"
        if line.starts_with('>') {
            in_quote = true;
            break;
        }
        // Outlook-style separator
        if line.trim() == "-----Original Message-----" || line.trim() == "________________________________" {
            in_quote = true;
            break;
        }
        if !in_quote {
            lines.push(line);
        }
    }

    lines.join("\n").trim().to_string()
}
```

---

## Batch Size and Scheduling

```rust
pub struct SyncConfig {
    pub batch_size:          u32,            // emails per sync cycle, default: 50
    pub sync_interval_secs:  u64,            // default: 1800 (30 min)
    pub max_llm_calls:       u32,            // budget guard, default: 200
    pub concurrent_fetches:  usize,          // concurrent message GETs, default: 10
}

// In trusty-daemon SyncScheduler
pub async fn run_sync_loop(
    config: SyncConfig,
    store: Arc<Store>,
    email_client: Arc<GmailClient>,
    extractor: Arc<Extractor>,
) {
    let mut interval = tokio::time::interval(
        Duration::from_secs(config.sync_interval_secs)
    );
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        match run_one_sync_cycle(&config, &store, &email_client, &extractor).await {
            Ok(report) => tracing::info!(
                emails = report.emails_processed,
                entities = report.entities_extracted,
                "Sync cycle complete"
            ),
            Err(e) => tracing::error!("Sync cycle failed: {e}"),
        }
    }
}

async fn run_one_sync_cycle(
    config: &SyncConfig,
    store: &Store,
    client: &GmailClient,
    extractor: &Extractor,
) -> TrustyResult<SyncReport> {
    let cursor = store.sessions().get_cursor(&client.account_id).await?;

    // Fetch emails
    let sync_result = client.sync_sent(cursor, config.batch_size).await?;
    if sync_result.emails.is_empty() {
        return Ok(SyncReport::empty());
    }

    // Extract entities (with LLM budget guard)
    let mut llm_calls = 0u32;
    let mut extracted = Vec::new();
    for email in &sync_result.emails {
        if llm_calls >= config.max_llm_calls {
            tracing::warn!("LLM budget exhausted mid-cycle, stopping");
            break;
        }
        // Spam check (1 LLM call)
        llm_calls += 1;
        if extractor.is_spam_or_newsletter(email).await? {
            continue;
        }
        // Extraction (1 LLM call)
        llm_calls += 1;
        if let Some(result) = extractor.extract_one(email).await? {
            extracted.push(result);
        }
    }

    // Persist
    let persist_report = persist_results(extracted, store, &extractor.engine).await?;

    // Update cursor
    store.sessions().set_cursor(&client.account_id, sync_result.new_cursor).await?;

    // Run memory decay
    let decay_report = MemoryManager::run_decay(store).await?;

    Ok(SyncReport {
        emails_processed:    sync_result.emails.len() as u32,
        entities_extracted:  persist_report.entities_written,
        relationships_written: persist_report.relationships_written,
        llm_calls_made:      llm_calls,
        memories_decayed:    decay_report.updated,
        memories_archived:   decay_report.archived,
    })
}
```

---

## Calendar Sync

Calendar data is fetched from the Google Calendar API and cached in SQLite's `calendar_cache` table. The cache is refreshed on every sync cycle (same 30-minute interval as email sync).

```rust
pub async fn sync_calendar(
    client: &CalendarClient,
    store: &Store,
) -> TrustyResult<u32> {
    let now = Utc::now();
    let seven_days = now + Duration::days(7);

    let events = client.get_events(
        &now.to_rfc3339(),
        &seven_days.to_rfc3339(),
    ).await?;

    // Replace all cached events for this account
    store.calendar().replace_events(&client.account_id, &events).await?;

    Ok(events.len() as u32)
}

// Calendar API call
pub async fn get_events(&self, time_min: &str, time_max: &str) -> TrustyResult<Vec<CalendarEvent>> {
    self.refresh_token_if_needed().await?;
    let token = self.store.get_token(&self.account_id).await?;

    let response = self.http
        .get("https://www.googleapis.com/calendar/v3/calendars/primary/events")
        .bearer_auth(&token.access_token)
        .query(&[
            ("timeMin",     time_min),
            ("timeMax",     time_max),
            ("singleEvents","true"),
            ("orderBy",     "startTime"),
            ("maxResults",  "100"),
        ])
        .send()
        .await?;

    let calendar_list: GoogleCalendarListResponse = response.json().await?;
    Ok(calendar_list.items.into_iter().map(CalendarEvent::from).collect())
}
```

---

## Error Handling Summary

| Error Condition                  | Response                                          |
|----------------------------------|---------------------------------------------------|
| 401 Unauthorized                 | Refresh access token, retry once                  |
| 403 rateLimitExceeded            | Exponential backoff (1s → 60s), up to 5 retries   |
| 429 Too Many Requests            | Same as rate limit                                |
| 410 Gone (historyId expired)     | Clear cursor, trigger full resync on next cycle   |
| 500/503 Google server error      | Retry with backoff, log error if persistent       |
| Network timeout (>30s)           | Log warning, skip cycle, retry next interval      |
| Token exchange failure           | Require user re-auth (`trusty-cli auth google`)   |
| LLM budget exhausted             | Stop mid-cycle, update cursor to safe position    |

```rust
pub async fn with_retry<F, T>(
    f: F,
    max_retries: u8,
) -> TrustyResult<T>
where
    F: Fn() -> futures::future::BoxFuture<'static, TrustyResult<T>>,
{
    let mut attempt = 0;
    let mut backoff = Duration::from_secs(1);

    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(TrustyError::RateLimit) | Err(TrustyError::ServerError(_)) => {
                attempt += 1;
                if attempt >= max_retries {
                    return Err(TrustyError::MaxRetriesExceeded);
                }
                tokio::time::sleep(backoff).await;
                backoff = std::cmp::min(backoff * 2, Duration::from_secs(60));
            }
            Err(e) => return Err(e),
        }
    }
}
```
