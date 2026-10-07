; Kill the bundled CLI before NSIS copies resources. Ignoring a locked
; `agent-doctor-cli.exe` leaves Ask/MCP/terminal using a stale or missing binary.
!macro NSIS_HOOK_PREINSTALL
  nsExec::ExecToLog 'taskkill /F /IM "agent-doctor-cli.exe" /T'
  Sleep 800
  ; Tauri copies resources into $INSTDIR (the directory the user picked,
  ; including a custom one). Older builds used $INSTDIR\resources. Clear both
  ; so a locked CLI is replaced instead of leaving MCP on a stale binary.
  IfFileExists "$INSTDIR\agent-doctor-cli.exe" 0 ad_cli_root_done
    Delete "$INSTDIR\agent-doctor-cli.exe"
    IfFileExists "$INSTDIR\agent-doctor-cli.exe" 0 ad_cli_root_done
      Delete /REBOOTOK "$INSTDIR\agent-doctor-cli.exe"
  ad_cli_root_done:
  IfFileExists "$INSTDIR\resources\agent-doctor-cli.exe" 0 ad_cli_preinstall_done
    Delete "$INSTDIR\resources\agent-doctor-cli.exe"
    IfFileExists "$INSTDIR\resources\agent-doctor-cli.exe" 0 ad_cli_preinstall_done
      Delete /REBOOTOK "$INSTDIR\resources\agent-doctor-cli.exe"
  ad_cli_preinstall_done:
!macroend

; Best-effort VC++ Redistributable when present next to the installer payload.
; Bundled CLI is also static-linked; this covers residual native deps / older builds.
!macro NSIS_HOOK_POSTINSTALL
  ReadRegDWord $0 HKLM "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" "Installed"
  ${If} $0 == 1
    DetailPrint "Visual C++ Redistributable already installed"
    Goto ad_vcredist_done
  ${EndIf}

  ${If} ${FileExists} "$INSTDIR\resources\vc_redist.x64.exe"
    DetailPrint "Installing Visual C++ Redistributable (quiet)…"
    CopyFiles "$INSTDIR\resources\vc_redist.x64.exe" "$TEMP\agent-doctor-vc_redist.x64.exe"
    ExecWait '"$TEMP\agent-doctor-vc_redist.x64.exe" /install /quiet /norestart' $0
    ${If} $0 == 0
      DetailPrint "Visual C++ Redistributable installed"
    ${ElseIf} $0 == 1638
      DetailPrint "Visual C++ Redistributable already present (newer)"
    ${Else}
      DetailPrint "Visual C++ Redistributable install exit code $0 (CLI is static-linked; continuing)"
    ${EndIf}
    Delete "$TEMP\agent-doctor-vc_redist.x64.exe"
    Delete "$INSTDIR\resources\vc_redist.x64.exe"
  ${EndIf}
  ad_vcredist_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'taskkill /F /IM "agent-doctor-cli.exe" /T'
  Sleep 400
!macroend
