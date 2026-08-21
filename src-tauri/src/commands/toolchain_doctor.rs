use crate::commands::CmdError;

/// Run the fixed, read-only host dependency probes. The implementation never
/// installs, upgrades, starts, or reconfigures a dependency.
#[tauri::command]
pub async fn get_toolchain_doctor_report(
) -> Result<ha_core::toolchain_doctor::ToolchainDoctorReport, CmdError> {
    Ok(ha_core::toolchain_doctor::diagnose_toolchain().await)
}
