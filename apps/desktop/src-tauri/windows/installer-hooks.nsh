; Tauri's NSIS installer inserts these macros (bundle.windows.nsis.installerHooks).

; HOST-13: an uninstall takes the `hatoba` command off the user's PATH. An update keeps it.
; Hatoba edits the PATH itself, because NSIS strings are too short to hold a long one.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $UpdateMode <> 1
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --uninstall-command'
  ${EndIf}
!macroend
