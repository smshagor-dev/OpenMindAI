!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "Installing OpenMindAI native llama runtime"
  ClearErrors
  CopyFiles /SILENT "$INSTDIR\resources\native-runtime\windows-x86_64\*.dll" "$INSTDIR"
  ${If} ${Errors}
    DetailPrint "ERROR: required native llama runtime DLLs could not be installed"
    SetErrorLevel 1
    Abort "Required native runtime DLL installation failed"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\llama.dll"
  Delete "$INSTDIR\ggml.dll"
  Delete "$INSTDIR\ggml-base.dll"
  Delete "$INSTDIR\ggml-cpu.dll"
  Delete "$INSTDIR\ggml-rpc.dll"
  Delete "$INSTDIR\ggml-vulkan.dll"
!macroend
