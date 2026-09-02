!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing OpenMindAI native llama runtime"
  CopyFiles /SILENT "$INSTDIR\resources\native-runtime\windows-x86_64\*.dll" "$INSTDIR"
  ${If} ${Errors}
    DetailPrint "WARNING: native llama runtime DLL copy reported an error; llama-server fallback remains available"
    ClearErrors
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\llama.dll"
  Delete "$INSTDIR\ggml.dll"
  Delete "$INSTDIR\ggml-base.dll"
  Delete "$INSTDIR\ggml-cpu.dll"
  Delete "$INSTDIR\ggml-rpc.dll"
!macroend
