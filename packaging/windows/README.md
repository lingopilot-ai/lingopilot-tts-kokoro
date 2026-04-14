This directory is a CI staging area for Windows release packaging.

`scripts/Stage-WindowsReleaseAssets.ps1` populates it before `scripts/Package-WindowsRelease.ps1` runs.

Expected staged inputs:

- `packaging/windows/onnxruntime.dll`
- `packaging/windows/kokoro-model/<exactly one *.onnx>`
- `packaging/windows/kokoro-model/<exactly one voices*.bin>`

These assets are intentionally not committed to the repository.
