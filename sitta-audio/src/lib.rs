//! Audio capture and pipeline for Sitta.
//!
//! Supports RTSP streams (via ffmpeg subprocess) and local audio devices.

pub mod chunk;
pub mod manager;
pub mod remote;
pub mod rtsp;
pub mod source;
pub mod wav;

/// Redact credentials from a URL for safe logging.
///
/// Replaces `user:pass@` in the authority with `***@`.
/// If the URL has no userinfo, returns it unchanged.
///
/// ```
/// assert_eq!(
///     sitta_audio::sanitize_url("rtsp://admin:secret@192.168.1.1/stream"),
///     "rtsp://***@192.168.1.1/stream"
/// );
/// assert_eq!(
///     sitta_audio::sanitize_url("rtsp://192.168.1.1/stream"),
///     "rtsp://192.168.1.1/stream"
/// );
/// ```
pub fn sanitize_url(url: &str) -> String {
    // Find the scheme separator (e.g., "rtsp://").
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let authority_start = scheme_end + 3;
    let rest = &url[authority_start..];

    // Find the `@` that separates userinfo from host.
    // Only look before the first `/` to avoid matching `@` in path/query.
    let path_start = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..path_start];
    if let Some(at_pos) = authority.find('@') {
        format!(
            "{}://***@{}",
            &url[..scheme_end],
            &rest[at_pos + 1..]
        )
    } else {
        url.to_string()
    }
}
