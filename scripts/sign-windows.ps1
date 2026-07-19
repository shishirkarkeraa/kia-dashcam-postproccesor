param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$ArtifactPath
)

$ErrorActionPreference = "Stop"
$certificate = $env:KIA_WINDOWS_PFX_PATH
$password = $env:WINDOWS_CERTIFICATE_PASSWORD
if (-not $certificate -or -not $password) {
    throw "KIA_WINDOWS_PFX_PATH and WINDOWS_CERTIFICATE_PASSWORD are required for production signing"
}

$kitsRoot = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin"
$signTool = Get-ChildItem -Path $kitsRoot -Filter signtool.exe -Recurse |
    Where-Object { $_.FullName -match "\\x64\\signtool\.exe$" } |
    Sort-Object FullName -Descending |
    Select-Object -First 1
if (-not $signTool) {
    throw "signtool.exe was not found in the Windows SDK"
}

& $signTool.FullName sign /f $certificate /p $password /fd SHA256 /td SHA256 /tr "http://timestamp.digicert.com" $ArtifactPath
if ($LASTEXITCODE -ne 0) {
    throw "signtool failed with exit code $LASTEXITCODE"
}
