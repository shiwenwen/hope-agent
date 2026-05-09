; Install Visual C++ 2015-2022 Redistributable — bundled native dependencies
; require MSVCP140_1.dll, which is absent on a clean Windows install. The
; Microsoft installer is idempotent on up-to-date systems (no-op exit), so
; we always run it after the main install rather than try to detect it from
; a 32-bit NSIS process (where $SYSDIR is redirected to SysWOW64).

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing Microsoft Visual C++ 2015-2022 Redistributable..."
  ExecWait '"$INSTDIR\resources\vc_redist.x64.exe" /install /quiet /norestart' $0
  Delete "$INSTDIR\resources\vc_redist.x64.exe"
  RMDir "$INSTDIR\resources"
!macroend
