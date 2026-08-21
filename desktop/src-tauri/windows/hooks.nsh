; Kill the bundled CLI before NSIS copies resources. Ignoring a locked
; `agent-doctor-cli.exe` leaves Ask/MCP/terminal using a stale or missing binary.
!macro NSIS_HOOK_PREINSTALL
  nsExec::ExecToLog 'taskkill /F /IM "agent-doctor-cli.exe" /T'
  Sleep 800
  IfFileExists "$INSTDIR\resources\agent-doctor-cli.exe" 0 ad_cli_preinstall_done
    Delete "$INSTDIR\resources\agent-doctor-cli.exe"
    IfFileExists "$INSTDIR\resources\agent-doctor-cli.exe" 0 ad_cli_preinstall_done
      Delete /REBOOTOK "$INSTDIR\resources\agent-doctor-cli.exe"
  ad_cli_preinstall_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'taskkill /F /IM "agent-doctor-cli.exe" /T'
  Sleep 400
!macroend
