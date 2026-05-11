/**
 * Current onboarding wizard version. Must stay in sync with the Rust
 * constant in `crates/ha-core/src/config/mod.rs` → `CURRENT_ONBOARDING_VERSION`.
 *
 * Bump both in the same commit only when existing users must re-walk the
 * flow. Optional steps that should appear only for new installs and manual
 * reruns should keep this value unchanged.
 *
 * App.tsx compares the persisted `completedVersion` against this and
 * launches the wizard when the user lags behind.
 */
export const CURRENT_ONBOARDING_VERSION = 1
