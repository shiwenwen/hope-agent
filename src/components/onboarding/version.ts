/**
 * Current onboarding wizard version. Must stay in sync with the Rust
 * constant in `crates/ha-core/src/config/mod.rs` → `CURRENT_ONBOARDING_VERSION`.
 *
 * Bump both in the same commit when adding a new step or otherwise
 * requiring existing users to re-walk the flow. App.tsx compares the
 * persisted `completedVersion` against this and launches the wizard when
 * the user lags behind.
 */
export const CURRENT_ONBOARDING_VERSION = 1
