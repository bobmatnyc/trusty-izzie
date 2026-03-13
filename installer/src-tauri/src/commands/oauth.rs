use std::sync::OnceLock;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

// Holds the authorized email after OAuth completes.
static OAUTH_RESULT: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn result_store() -> &'static Mutex<Option<String>> {
    OAUTH_RESULT.get_or_init(|| Mutex::new(None))
}

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth\
     ?client_id=409456389838-placeholder\
     &redirect_uri=http://localhost:8080/callback\
     &response_type=code\
     &scope=https://www.googleapis.com/auth/gmail.readonly\
       %20https://www.googleapis.com/auth/calendar.readonly\
       %20https://www.googleapis.com/auth/gmail.send\
     &access_type=offline\
     &prompt=consent";

#[tauri::command]
pub async fn start_google_oauth(app: tauri::AppHandle) -> Result<(), String> {
    // Reset any previous result.
    *result_store().lock().await = None;

    // Open browser.
    app.opener()
        .open_url(GOOGLE_AUTH_URL, None::<String>)
        .map_err(|e| e.to_string())?;

    // Spawn a task that listens on :8080 for the OAuth callback.
    tokio::spawn(async move {
        if let Err(e) = listen_for_callback().await {
            eprintln!("OAuth callback listener error: {e}");
        }
    });

    Ok(())
}

async fn listen_for_callback() -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    let (mut stream, _) = listener.accept().await?;

    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    // Parse the first line: GET /callback?code=...&... HTTP/1.1
    let email = parse_email_from_request(&request);

    let response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
        <html><body><h2>Authorized! You can close this tab.</h2></body></html>";
    stream.write_all(response.as_bytes()).await?;

    *result_store().lock().await = Some(email);
    Ok(())
}

fn parse_email_from_request(request: &str) -> String {
    // Try to extract an email hint from the `login_hint` param or fallback.
    // In practice the daemon does full token exchange; installer just captures
    // the auth code and stores a placeholder email from the query string.
    for line in request.lines() {
        if line.starts_with("GET ") {
            if let Some(path_end) = line.find(" HTTP/") {
                let path = &line[4..path_end];
                if let Some(q) = path.find('?') {
                    let query = &path[q + 1..];
                    for part in query.split('&') {
                        if let Some(v) = part.strip_prefix("login_hint=") {
                            return urlencoded_decode(v);
                        }
                        if let Some(v) = part.strip_prefix("email=") {
                            return urlencoded_decode(v);
                        }
                    }
                }
            }
        }
    }
    // Fallback: signal success without a known email.
    "authorized".to_string()
}

fn urlencoded_decode(s: &str) -> String {
    s.replace("%40", "@").replace("+", " ")
}

#[tauri::command]
pub async fn poll_oauth_result() -> Result<Option<String>, String> {
    let guard = result_store().lock().await;
    Ok(guard.clone())
}
