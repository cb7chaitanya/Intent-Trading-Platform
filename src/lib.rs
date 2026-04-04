pub mod config;
pub mod gateway;
pub mod metrics;

// JWT utilities for the gateway binary
pub mod jwt {
    pub use crate::_jwt_impl::*;
}

#[path = "auth/jwt.rs"]
mod _jwt_impl;

#[path = "auth/key_rotation.rs"]
mod key_rotation;

// API key service for the gateway binary
#[path = "api_keys/service.rs"]
pub mod api_key_service;
