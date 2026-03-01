# API Design

## Overview

trusty-izzie exposes its functionality through two interfaces:

1. **HTTP REST API** — implemented with Axum in `trusty-api`. Suitable for web clients, scripts, third-party integrations, and remote access over LAN.
2. **IPC Protocol** — Unix socket with JSON messages. Used by `trusty-cli` and `trusty-tui` for low-latency local access without HTTP overhead.

Both interfaces share the same underlying `ChatEngine`, `Store`, and `EmbeddingEngine` instances managed by `trusty-daemon`.

---

## Authentication

trusty-izzie is a single-user local application. Authentication is handled by a local API key stored in SQLite's `config` table.

**Key generation (on first run):**
```rust
let api_key = format!("ti_{}", hex::encode(&rand::thread_rng().gen::<[u8; 24]>()));
store.config_kv().set("api_key", &api_key).await?;
```

**HTTP authentication:** The key is passed in the `Authorization` header (Bearer) or `X-API-Key` header:
```
Authorization: Bearer ti_abc123...
X-API-Key: ti_abc123...
```

**IPC authentication:** The Unix socket is owned by the user's OS user (mode 600). No token needed — access is controlled by filesystem permissions.

**CLI authentication:** The CLI reads the API key from `~/.trusty-izzie/config` or the `TRUSTY_API_KEY` environment variable.

**Axum middleware:**
```rust
async fn require_api_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    let provided_key = headers
        .get("x-api-key")
        .or_else(|| headers.get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok()?.strip_prefix("Bearer ").map(HeaderValue::from_str).ok()?.as_ref()))
        .and_then(|v| v.to_str().ok());

    match provided_key {
        Some(key) if state.valid_api_key(key) => next.run(request).await,
        _ => (StatusCode::UNAUTHORIZED, "Invalid or missing API key").into_response(),
    }
}
```

---

## HTTP REST API

### Router Structure

```rust
pub fn create_router(state: AppState) -> Router {
    let api = Router::new()
        // Chat
        .route("/chat",                post(chat_handler))
        .route("/chat/sessions",       get(list_sessions_handler))
        // Entities
        .route("/entities",            get(list_entities_handler))
        .route("/entities/:id",        get(get_entity_handler))
        // Graph
        .route("/graph/neighbors/:id", get(get_neighbors_handler))
        // Memories
        .route("/memories",            get(list_memories_handler))
        // Sync
        .route("/sync",                post(trigger_sync_handler))
        .route("/status",              get(status_handler))
        // Auth
        .route("/auth/google",         post(start_google_auth_handler))
        .route("/auth/google/callback",get(google_auth_callback_handler))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_api_key));

    Router::new()
        .nest("/api", api)
        .layer(
            ServiceBuilder::new()
                .layer(TraceLayer::new_for_http())
                .layer(CorsLayer::permissive())  // local-only, permissive is fine
        )
        .with_state(state)
}
```

---

### `POST /api/chat`

Send a message and receive a streaming SSE response.

**Request:**
```json
{
  "session_id": "uuid-or-null",
  "message":    "Who does Alice work with at Acme?",
  "options": {
    "model":           "anthropic/claude-sonnet-4-5",
    "max_tokens":      2048,
    "disable_tools":   false,
    "disable_memory_save": false
  }
}
```

If `session_id` is null or omitted, a new session is created automatically.

**Response:** `text/event-stream` (SSE)

```
Content-Type: text/event-stream
Cache-Control: no-cache
X-Session-Id: <new or existing session UUID>

data: {"type":"token","token":"Based"}

data: {"type":"token","token":" on"}

data: {"type":"token","token":" your"}

data: {"type":"tool_call","name":"find_related","args":{"entity_id":"...","depth":1}}

data: {"type":"tool_result","result":{"neighbors":[...]}}

data: {"type":"token","token":"Alice works with..."}

data: {"type":"done","session_id":"uuid","message":"Alice works with Bob...","tokens_used":387,"memories_saved":1}
```

**Error response:**
```
data: {"type":"error","error":"LLM API unavailable"}
```

**Rust handler:**
```rust
pub async fn chat_handler(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let session_id = match req.session_id {
        Some(id) => id,
        None => {
            let session = state.chat_engine
                .new_session(&state.config.user_id, &state.store)
                .await
                .unwrap();
            session.id
        }
    };

    let (tx, rx) = mpsc::channel::<ChatEvent>(64);
    let state_clone = state.clone();
    let session_id_clone = session_id.clone();
    let message = req.message.clone();
    let opts = req.options.unwrap_or_default();

    tokio::spawn(async move {
        state_clone.chat_engine
            .send_message(&session_id_clone, &message, &state_clone.store, opts, tx)
            .await
            .unwrap_or_else(|e| tracing::error!("Chat error: {e}"));
    });

    let stream = ReceiverStream::new(rx).map(|event| {
        let data = serde_json::to_string(&event).unwrap_or_default();
        Ok::<Event, Infallible>(Event::default().data(data))
    });

    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response();

    response.headers_mut().insert(
        "X-Session-Id",
        HeaderValue::from_str(&session_id).unwrap(),
    );
    response
}
```

---

### `GET /api/chat/sessions`

List all chat sessions for the user.

**Query params:**
| Param  | Type    | Default | Description             |
|--------|---------|---------|-------------------------|
| limit  | integer | 20      | Max sessions to return  |
| offset | integer | 0       | Pagination offset        |

**Response:**
```json
{
  "sessions": [
    {
      "id":           "uuid",
      "title":        "Who does Alice work with?",
      "created_at":   "2026-01-15T10:30:00Z",
      "last_active_at": "2026-01-15T10:35:00Z",
      "message_count": 6
    }
  ],
  "total": 42,
  "limit": 20,
  "offset": 0
}
```

---

### `GET /api/entities`

List entities with pagination and filtering.

**Query params:**
| Param       | Type    | Default | Description                                         |
|-------------|---------|---------|-----------------------------------------------------|
| type        | string  | (all)   | Filter: Person, Company, Project, Tool, Topic, Location |
| q           | string  | (none)  | Hybrid search query (BM25 + vector)                 |
| limit       | integer | 20      | Max results                                         |
| offset      | integer | 0       | Pagination offset                                   |
| min_seen    | integer | 1       | Minimum times seen in emails                        |

**Response:**
```json
{
  "entities": [
    {
      "id":           "uuid",
      "name":         "Alice Johnson",
      "entity_type":  "Person",
      "confidence":   0.92,
      "seen_count":   14,
      "attributes": {
        "email":   "alice@acme.com",
        "role":    "Engineering Manager",
        "company": "Acme Corp"
      },
      "first_seen": "2025-11-01T00:00:00Z",
      "last_seen":  "2026-02-28T00:00:00Z"
    }
  ],
  "total": 156,
  "limit": 20,
  "offset": 0
}
```

---

### `GET /api/entities/{id}`

Get full details for a single entity, including its graph relationships.

**Response:**
```json
{
  "id":           "uuid",
  "name":         "Alice Johnson",
  "entity_type":  "Person",
  "confidence":   0.92,
  "seen_count":   14,
  "attributes": {
    "email":   "alice@acme.com",
    "role":    "Engineering Manager",
    "company": "Acme Corp"
  },
  "relationships": [
    {
      "type":       "WORKS_FOR",
      "direction":  "outgoing",
      "target": {
        "id":   "company-uuid",
        "name": "Acme Corp",
        "type": "Company"
      },
      "confidence": 0.95,
      "evidence":   "Signed email as 'Engineering Manager at Acme Corp'",
      "first_seen": "2025-11-01T00:00:00Z",
      "last_seen":  "2026-02-28T00:00:00Z"
    },
    {
      "type":      "WORKS_WITH",
      "direction": "outgoing",
      "target": {
        "id":   "person-uuid-2",
        "name": "Bob Chen",
        "type": "Person"
      },
      "confidence": 0.87,
      "evidence":   "Coordinating on the Prometheus project rollout"
    }
  ],
  "first_seen": "2025-11-01T00:00:00Z",
  "last_seen":  "2026-02-28T00:00:00Z"
}
```

**Error (not found):**
```json
{ "error": "Entity not found", "id": "uuid" }
```
Status: 404

---

### `GET /api/graph/neighbors/{id}`

Get the graph neighborhood of an entity.

**Query params:**
| Param             | Type    | Default | Description                |
|-------------------|---------|---------|----------------------------|
| depth             | integer | 1       | Traversal depth (max: 2)   |
| relationship_type | string  | (all)   | Filter by relationship type |

**Response:**
```json
{
  "center": {
    "id": "uuid", "name": "Alice Johnson", "type": "Person"
  },
  "nodes": [
    { "id": "uuid-2", "name": "Acme Corp", "type": "Company" },
    { "id": "uuid-3", "name": "Bob Chen",  "type": "Person"  },
    { "id": "uuid-4", "name": "Prometheus","type": "Project" }
  ],
  "edges": [
    { "from": "uuid", "to": "uuid-2", "type": "WORKS_FOR", "confidence": 0.95 },
    { "from": "uuid", "to": "uuid-3", "type": "WORKS_WITH","confidence": 0.87 },
    { "from": "uuid", "to": "uuid-4", "type": "WORKS_ON",  "confidence": 0.91 }
  ]
}
```

---

### `GET /api/memories`

List memories with optional filtering.

**Query params:**
| Param      | Type    | Default | Description                                         |
|------------|---------|---------|-----------------------------------------------------|
| category   | string  | (all)   | preference, fact, relationship, decision, event, sentiment, reminder |
| q          | string  | (none)  | Hybrid search query                                 |
| min_strength | float | 0.05   | Minimum strength threshold                         |
| limit      | integer | 20      | Max results                                         |
| offset     | integer | 0       | Pagination offset                                   |

**Response:**
```json
{
  "memories": [
    {
      "id":           "uuid",
      "content":      "Alice prefers async communication over meetings",
      "category":     "preference",
      "confidence":   0.92,
      "importance":   0.7,
      "strength":     0.84,
      "created_at":   "2026-01-10T14:22:00Z",
      "last_accessed":"2026-02-28T09:15:00Z",
      "entity_refs":  ["alice-entity-uuid"]
    }
  ],
  "total": 87,
  "limit": 20,
  "offset": 0
}
```

---

### `POST /api/sync`

Trigger a manual sync cycle immediately.

**Request body:** empty or `{}`

**Response:**
```json
{
  "status":                "started",
  "job_id":               "uuid",
  "message":              "Sync cycle started. Poll /api/status for progress."
}
```

If a sync is already running:
```json
{ "status": "already_running", "started_at": "2026-03-01T12:00:00Z" }
```
Status: 409

---

### `GET /api/status`

Health check and daemon status.

**Response:**
```json
{
  "status":           "healthy",
  "version":          "0.1.0",
  "uptime_secs":      86400,
  "sync": {
    "is_syncing":          false,
    "last_sync_at":        "2026-03-01T11:30:00Z",
    "next_sync_at":        "2026-03-01T12:00:00Z",
    "last_sync_emails":    12,
    "last_sync_entities":  5
  },
  "storage": {
    "entities_total":  234,
    "memories_total":  89,
    "sessions_total":  17
  },
  "accounts": [
    {
      "account_id": "user@gmail.com",
      "connected":  true,
      "cursor":     "28742983"
    }
  ]
}
```

---

### `POST /api/auth/google`

Initiate the Google OAuth2 flow. Opens a browser window on the server machine.

**Request body:** empty or `{}`

**Response:**
```json
{
  "status":  "pending",
  "message": "Browser opened. Complete authentication in the browser window."
}
```

This endpoint blocks until the OAuth callback is received (max 5 minutes), then responds with success or failure.

**Success response:**
```json
{
  "status":     "connected",
  "account_id": "user@gmail.com",
  "message":    "Google account connected successfully."
}
```

**Error response:**
```json
{ "status": "error", "error": "OAuth timeout — no browser response within 5 minutes" }
```
Status: 408

---

### `GET /api/auth/google/callback`

OAuth callback endpoint. Called by Google after user authorization. This is registered as the redirect URI in the Google Cloud Console.

**Query params (set by Google):**
- `code` — authorization code
- `state` — CSRF protection state value

This endpoint is consumed by the local redirect server, not directly by API clients. It returns an HTML success page to the browser.

**Success response (HTML):**
```html
<!DOCTYPE html>
<html>
<body>
<h2>Authentication successful</h2>
<p>You can close this window. trusty-izzie is now connected to your Google account.</p>
</body>
</html>
```

---

## IPC Protocol (Unix Socket)

`trusty-cli` and `trusty-tui` communicate with the daemon via a Unix domain socket at `{data_dir}/trusty.sock`.

### Message Format

All messages are newline-delimited JSON (`\n` after each complete message).

**Request envelope:**
```json
{
  "id":     "request-uuid",
  "method": "chat.send",
  "params": { ... }
}
```

**Response envelope:**
```json
{
  "id":     "request-uuid",
  "result": { ... }
}
```

**Error envelope:**
```json
{
  "id":    "request-uuid",
  "error": { "code": 404, "message": "Entity not found" }
}
```

**Streaming envelope (one per token/event):**
```json
{ "id": "request-uuid", "event": "token",  "data": { "token": "Hello" } }
{ "id": "request-uuid", "event": "done",   "data": { "message": "...", "tokens_used": 342 } }
```

### IPC Methods

| Method                          | Params                                   | Response           |
|---------------------------------|------------------------------------------|--------------------|
| `chat.send`                     | `{session_id, message, options}`        | streaming events   |
| `chat.list_sessions`            | `{limit, offset}`                        | sessions array     |
| `chat.new_session`              | `{}`                                     | session            |
| `entities.list`                 | `{type?, q?, limit, offset}`             | entities array     |
| `entities.get`                  | `{id}`                                   | entity + relations |
| `graph.neighbors`               | `{id, depth, relationship_type?}`        | nodes + edges      |
| `memories.list`                 | `{category?, q?, min_strength?, limit}`  | memories array     |
| `sync.trigger`                  | `{}`                                     | job_id + status    |
| `status.get`                    | `{}`                                     | status object      |
| `auth.google.start`             | `{}`                                     | pending message    |

### IPC Client Implementation (in trusty-cli)

```rust
pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    pub async fn connect(socket_path: &Path) -> TrustyResult<Self> {
        let stream = UnixStream::connect(socket_path)
            .await
            .map_err(|_| TrustyError::DaemonNotRunning)?;
        Ok(Self { stream })
    }

    pub async fn call(&mut self, method: &str, params: serde_json::Value) -> TrustyResult<serde_json::Value> {
        let request = IpcRequest {
            id:     Uuid::new_v4().to_string(),
            method: method.to_string(),
            params,
        };

        self.send(&request).await?;

        // Read until we get a response with matching id
        loop {
            let response = self.recv().await?;
            if response.id == request.id {
                return response.result
                    .ok_or_else(|| TrustyError::IpcError(
                        response.error.map(|e| e.message).unwrap_or_default()
                    ));
            }
        }
    }

    pub async fn call_streaming(
        &mut self,
        method: &str,
        params: serde_json::Value,
        tx: mpsc::Sender<serde_json::Value>,
    ) -> TrustyResult<()> {
        let request = IpcRequest {
            id:     Uuid::new_v4().to_string(),
            method: method.to_string(),
            params,
        };

        self.send(&request).await?;

        loop {
            let msg = self.recv_raw().await?;
            let envelope: serde_json::Value = serde_json::from_str(&msg)?;

            if envelope["id"] == request.id {
                match envelope.get("event").and_then(|e| e.as_str()) {
                    Some("done") | Some("error") => {
                        tx.send(envelope["data"].clone()).await.ok();
                        break;
                    }
                    Some(_) => {
                        tx.send(envelope["data"].clone()).await.ok();
                    }
                    None => break,
                }
            }
        }

        Ok(())
    }
}
```

### Fallback: Direct Library Calls

If the daemon is not running, `trusty-cli` falls back to direct library calls (bypassing IPC):

```rust
async fn run_chat_command(args: ChatArgs) -> TrustyResult<()> {
    let config = AppConfig::load()?;
    let socket_path = config.socket_path();

    if socket_path.exists() {
        // Try IPC first
        match IpcClient::connect(&socket_path).await {
            Ok(mut client) => {
                return run_chat_via_ipc(&mut client, args).await;
            }
            Err(_) => {
                tracing::warn!("Daemon socket exists but connection failed, using direct mode");
            }
        }
    }

    // Direct mode: initialize store inline (slower startup)
    let store = Arc::new(Store::new(&config.storage).await?);
    let engine = Arc::new(EmbeddingEngine::new(&config.embedding)?);
    let chat_engine = ChatEngine::new(&config.chat);
    run_chat_direct(&chat_engine, &store, &engine, args).await
}
```

---

## API State

```rust
#[derive(Clone)]
pub struct AppState {
    pub store:        Arc<Store>,
    pub chat_engine:  Arc<ChatEngine>,
    pub engine:       Arc<EmbeddingEngine>,
    pub search:       Arc<HybridSearchEngine>,
    pub memory_mgr:   Arc<MemoryManager>,
    pub config:       Arc<AppConfig>,
    api_key_hash:     String,  // bcrypt hash of API key
}

impl AppState {
    pub fn valid_api_key(&self, provided: &str) -> bool {
        bcrypt::verify(provided, &self.api_key_hash).unwrap_or(false)
    }
}
```

---

## Error Responses

All HTTP error responses follow a consistent JSON envelope:

```json
{
  "error":   "Human-readable error message",
  "code":    "ENTITY_NOT_FOUND",
  "details": { }
}
```

| HTTP Status | Condition                                 |
|-------------|-------------------------------------------|
| 400         | Invalid request body or params            |
| 401         | Missing or invalid API key                |
| 404         | Entity/session/resource not found         |
| 408         | OAuth timeout                             |
| 409         | Conflict (e.g., sync already running)     |
| 422         | Valid JSON but semantically invalid       |
| 429         | Rate limited (future: per-key throttling) |
| 500         | Internal server error                     |
| 503         | Storage backend unavailable               |

---

## HTTP Server Configuration

```rust
pub struct ApiConfig {
    pub host:         String,   // default: "127.0.0.1" (local only)
    pub port:         u16,      // default: 7474
    pub request_timeout_secs: u64, // default: 120
}
```

```toml
[api]
host = "127.0.0.1"
port = 7474
request_timeout_secs = 120
```

The server binds to `127.0.0.1` by default. Users who want LAN access must explicitly set `host = "0.0.0.0"` and understand the security implications (protected only by the API key).

---

## CLI Command Reference

`trusty-cli` maps directly to IPC methods:

```bash
# Chat
trusty-cli chat "Who does Alice work with?"
trusty-cli chat --session <uuid> "What did we decide about the API?"
trusty-cli sessions                          # list sessions

# Entities
trusty-cli entities list
trusty-cli entities list --type Person
trusty-cli entities list --search "machine learning"
trusty-cli entities get <id>
trusty-cli graph neighbors <id>

# Memories
trusty-cli memories list
trusty-cli memories list --category preference

# Sync and status
trusty-cli sync
trusty-cli status

# Auth
trusty-cli auth google

# Config
trusty-cli config get default_chat_model
trusty-cli config set default_sync_interval 900

# Output format
trusty-cli --json entities list   # JSON output for scripting
```
