; Install Visual C++ 2015-2022 Redistributable — bundled native dependencies
; require MSVCP140_1.dll, which is absent on a clean Windows install. Always
; run the official installer rather than try to detect from a 32-bit NSIS
; process (where $SYSDIR is redirected to SysWOW64). Accept 0 / 1638 / 3010
; as success per Microsoft conventions; surface any other code so the user
; knows to install manually instead of silently shipping a broken setup.

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing Microsoft Visual C++ 2015-2022 Redistributable..."
  ExecWait '"$INSTDIR\resources\vc_redist.x64.exe" /install /quiet /norestart' $0
  ${If} $0 = 0
    DetailPrint "VC++ Redistributable installed."
  ${ElseIf} $0 = 1638
    DetailPrint "VC++ Redistributable already up to date."
  ${ElseIf} $0 = 3010
    DetailPrint "VC++ Redistributable installed (reboot recommended after Hope Agent setup)."
  ${Else}
    DetailPrint "VC++ Redistributable installation failed (exit code $0)."
    MessageBox MB_OK|MB_ICONEXCLAMATION "Failed to install Microsoft Visual C++ 2015-2022 Redistributable (exit code $0).$\r$\n$\r$\nHope Agent has been installed but may fail to start with 'MSVCP140_1.dll not found'. Please install the runtime manually from:$\r$\nhttps://aka.ms/vs/17/release/vc_redist.x64.exe"
  ${EndIf}
  Delete "$INSTDIR\resources\vc_redist.x64.exe"
  RMDir "$INSTDIR\resources"
!macroend
