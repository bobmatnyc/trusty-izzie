//! Maps link generation with configurable provider (Google Maps or Apple Maps).

/// Generate an HTML anchor tag linking an address to a maps provider.
///
/// `provider` should be `"google"` or `"apple"` (case-insensitive).
/// Defaults to Google Maps for any unrecognized value.
///
/// Returns e.g. `<a href="https://maps.google.com/?q=100+Park+Ave">100 Park Ave</a>`
pub fn maps_link(address: &str, provider: &str) -> String {
    let encoded = urlencoding::encode(address);
    let url = match provider.to_lowercase().as_str() {
        "apple" => format!("https://maps.apple.com/?q={}", encoded),
        _ => format!("https://maps.google.com/?q={}", encoded),
    };
    format!("<a href=\"{}\">{}</a>", url, address)
}
