//! Deterministic scene evaluation and a shared-device GPU renderer.
//!
//! Scene and data types deliberately contain no graphics backend handles.

pub mod animation;
pub mod data;
pub mod eval;
pub mod export;
pub mod host;
mod output;
pub mod reference;
pub mod render;
pub mod scene;
mod text;
pub mod time;
