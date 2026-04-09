//! Shared accessors for oc-core globals, wrapped as `Result<_, AppError>`
//! so handlers can use `?` instead of re-implementing the unwrap boilerplate
//! in every route file.

use std::sync::Arc;

use oc_core::channel::{ChannelDB, ChannelRegistry};
use oc_core::cron::CronDB;
use oc_core::session::SessionDB;
use oc_core::AppState;

use crate::error::AppError;

pub fn app_state() -> Result<&'static Arc<AppState>, AppError> {
    oc_core::get_app_state().ok_or_else(|| AppError::internal("AppState not initialized"))
}

pub fn session_db() -> Result<&'static Arc<SessionDB>, AppError> {
    oc_core::get_session_db().ok_or_else(|| AppError::internal("Session DB not initialized"))
}

pub fn cron_db() -> Result<&'static Arc<CronDB>, AppError> {
    oc_core::get_cron_db().ok_or_else(|| AppError::internal("Cron DB not initialized"))
}

pub fn channel_registry() -> Result<&'static Arc<ChannelRegistry>, AppError> {
    oc_core::get_channel_registry()
        .ok_or_else(|| AppError::internal("Channel registry not initialized"))
}

pub fn channel_db() -> Result<&'static Arc<ChannelDB>, AppError> {
    oc_core::get_channel_db().ok_or_else(|| AppError::internal("Channel DB not initialized"))
}
