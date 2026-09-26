; Candlewick starts at login through the Run value named after the product,
; which the uninstaller already removes (except when updating). Task Manager
; keeps its own on/off switch for that value; it goes with it.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "${PRODUCTNAME}"
  ${EndIf}
!macroend
