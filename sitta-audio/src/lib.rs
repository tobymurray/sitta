//! Audio capture and pipeline for Sitta.
//!
//! Supports RTSP streams (via ffmpeg subprocess) and local audio devices.

pub mod chunk;
pub mod remote;
pub mod rtsp;
pub mod source;
