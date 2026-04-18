use serde::Deserialize;

/// Configuration for a single audio source.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum SourceConfig {
    /// RTSP stream captured via ffmpeg.
    #[serde(rename = "rtsp")]
    Rtsp(RtspSourceConfig),
    /// Local sound card (not yet implemented).
    #[serde(rename = "local")]
    Local(LocalSourceConfig),
    /// Remote Sitta instance audio stream.
    #[serde(rename = "remote")]
    Remote(RemoteSourceConfig),
}

impl SourceConfig {
    pub fn name(&self) -> &str {
        match self {
            Self::Rtsp(c) => &c.name,
            Self::Local(c) => &c.name,
            Self::Remote(c) => &c.name,
        }
    }
}

/// Remote audio source: connects to another Sitta instance's PCM stream.
#[derive(Debug, Clone, Deserialize)]
pub struct RemoteSourceConfig {
    /// Display name for this source.
    pub name: String,
    /// Full URL to the remote audio stream endpoint,
    /// e.g., "http://192.168.1.10:8080/api/v1/audio/stream/north_feeder".
    pub url: String,
    /// Seconds to wait before reconnecting after a failure.
    #[serde(default = "default_reconnect_seconds")]
    pub reconnect_seconds: u64,
}

/// RTSP stream configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct RtspSourceConfig {
    /// Human-readable source name (e.g., "north_paddock").
    pub name: String,
    /// Full RTSP URL including credentials if needed.
    pub url: String,
    /// RTSP transport protocol.
    #[serde(default)]
    pub transport: Transport,
    /// Desired sample rate in Hz. ffmpeg resamples if the source differs.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    /// Number of audio channels.
    #[serde(default = "default_channels")]
    pub channels: u16,
    /// Seconds to wait before reconnecting after a failure.
    #[serde(default = "default_reconnect_seconds")]
    pub reconnect_seconds: u64,
    /// ffmpeg socket timeout in seconds.
    #[serde(default = "default_timeout_seconds")]
    pub timeout_seconds: u64,
}

/// Local audio device configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct LocalSourceConfig {
    /// Human-readable source name.
    pub name: String,
    /// ALSA device name or substring to match.
    pub device: String,
    /// Desired sample rate in Hz.
    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,
    /// Number of audio channels.
    #[serde(default = "default_channels")]
    pub channels: u16,
}

/// RTSP transport protocol.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    #[default]
    Tcp,
    Udp,
}

impl Transport {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }
}

fn default_sample_rate() -> u32 {
    48000
}
fn default_channels() -> u16 {
    1
}
fn default_reconnect_seconds() -> u64 {
    5
}
fn default_timeout_seconds() -> u64 {
    10
}
