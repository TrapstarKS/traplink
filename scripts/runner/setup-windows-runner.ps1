#Requires -Version 5.1
[CmdletBinding()]
param(
  [string]$Token,
  [string]$Repository = "TrapstarKS/traplink",
  [string]$RunnerDir = "C:\actions-runner-traplink",
  [string]$Labels = "traplink",
  [string]$RunnerName = "$($env:COMPUTERNAME)-traplink"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
  Write-Host "This script needs an elevated PowerShell (Run as administrator)." -ForegroundColor Yellow
  Write-Host "Reason: it installs the runner as a Windows service and grants access to the NETWORK SERVICE account."
  exit 1
}
if ([Environment]::Is64BitOperatingSystem -eq $false) { throw "The TrapLink runner requires Windows x64." }

if (-not $Token) {
  $secure = Read-Host "Runner registration token (gh api -X POST repos/$Repository/actions/runners/registration-token --jq .token)" -AsSecureString
  $Token = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure))
}
if (-not $Token) { throw "Missing registration token." }

if (Test-Path (Join-Path $RunnerDir ".runner")) {
  Write-Host "A runner is already configured in $RunnerDir." -ForegroundColor Yellow
  Write-Host "To reconfigure, remove it first:  cd $RunnerDir; .\config.cmd remove --token <removal token>"
  exit 1
}

$warnings = @()
function Test-Tool($name, $hint) {
  if (-not (Get-Command $name -ErrorAction SilentlyContinue)) { $script:warnings += "$name not found in the system PATH. $hint" }
}
$machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
$env:Path = "$machinePath;C:\rust\cargo\bin"
Test-Tool "node" "Install Node.js 22 LTS for all users (winget install OpenJS.NodeJS.LTS --scope machine)."
Test-Tool "pnpm" "Install into the Node directory: npm install -g --prefix `"$env:ProgramFiles\nodejs`" pnpm@10.10.0"
Test-Tool "gh" "Install GitHub CLI for all users (winget install GitHub.cli --scope machine)."
Test-Tool "git" "Install Git for Windows (winget install Git.Git --scope machine)."
if (-not (Test-Path "C:\rust\cargo\bin\cargo.exe")) { $warnings += "Rust not found in C:\rust. See the Rust setup section in scripts/runner/README.md." }
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$vc = if (Test-Path $vswhere) { & $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath } else { $null }
if (-not $vc) { $warnings += "Visual Studio Build Tools with the 'Desktop development with C++' workload not found (winget install Microsoft.VisualStudio.2022.BuildTools)." }
if ($warnings.Count) {
  Write-Host "Toolchain warnings (the runner will be installed, but builds will fail until these are fixed):" -ForegroundColor Yellow
  $warnings | ForEach-Object { Write-Host "  - $_" -ForegroundColor Yellow }
}

$headers = @{ "User-Agent" = "traplink-runner-setup"; "Accept" = "application/vnd.github+json" }
$release = Invoke-RestMethod -Uri "https://api.github.com/repos/actions/runner/releases/latest" -Headers $headers
$version = $release.tag_name.TrimStart("v")
$assetName = "actions-runner-win-x64-$version.zip"
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1
if (-not $asset) { throw "File $assetName not found in actions/runner release $($release.tag_name)." }
if ($release.body -notmatch "<!-- BEGIN SHA win-x64 -->\s*([0-9a-fA-F]{64})\s*<!-- END SHA win-x64 -->") {
  throw "Could not read the published win-x64 SHA-256 from the $($release.tag_name) release notes. Aborted without installing."
}
$expected = $Matches[1].ToLowerInvariant()

$download = Join-Path ([IO.Path]::GetTempPath()) ("traplink-runner-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $download | Out-Null
$zip = Join-Path $download $assetName
Write-Host "Downloading $assetName ..."
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zip -UseBasicParsing -Headers @{ "User-Agent" = "traplink-runner-setup" }
$actual = (Get-FileHash -Path $zip -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne $expected) {
  Remove-Item -Recurse -Force $download
  throw "SHA-256 mismatch (expected $expected, got $actual). Aborted without installing."
}
Write-Host "SHA-256 verified: $actual" -ForegroundColor Green

New-Item -ItemType Directory -Force -Path $RunnerDir | Out-Null
Expand-Archive -Path $zip -DestinationPath $RunnerDir -Force
Remove-Item -Recurse -Force $download

Push-Location $RunnerDir
try {
  & .\config.cmd --unattended `
    --url "https://github.com/$Repository" `
    --token $Token `
    --name $RunnerName `
    --labels $Labels `
    --work "_work" `
    --runasservice
  $configExit = $LASTEXITCODE
} finally {
  Pop-Location
  $Token = $null
}

$service = Get-Service "actions.runner.*.$RunnerName" -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $service) { throw "config.cmd failed with exit code $configExit and the service was not created." }
cmd /c "sc config `"$($service.Name)`" obj= `"NT AUTHORITY\NetworkService`" password= `"`"" | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Could not set the account for service $($service.Name)." }
if ($service.Status -ne "Running") { Start-Service $service.Name }
if ((Get-Service $service.Name).Status -ne "Running") { throw "Service $($service.Name) did not start. See $RunnerDir\_diag." }

Write-Host ""
Write-Host "Runner '$RunnerName' installed as a service (account NT AUTHORITY\NETWORK SERVICE)." -ForegroundColor Green
Write-Host "Labels: self-hosted, Windows, X64, $Labels"
Write-Host "If you change the system PATH later, restart the service: Get-Service actions.runner.* | Restart-Service"
