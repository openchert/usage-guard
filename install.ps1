$ErrorActionPreference = 'Stop'

$Repo = 'openchert/usage-guard'
$ApiUrl = "https://api.github.com/repos/$Repo/releases/latest"
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $HOME 'bin' }

$arch = if ([Environment]::Is64BitOperatingSystem) { 'x64' } else { 'x86' }
if ($arch -ne 'x64') {
  throw "Unsupported architecture: $arch. Available release asset: windows-x64"
}

$assetName = 'usage-guard-windows-x64.zip'

Write-Host 'Fetching latest release metadata...'
$release = Invoke-RestMethod -Uri $ApiUrl
$asset = $release.assets | Where-Object { $_.name -eq $assetName } | Select-Object -First 1

if (-not $asset) {
  throw "Could not find $assetName in latest release. Check https://github.com/$Repo/releases"
}

$tmp = Join-Path $env:TEMP ("usageguard-" + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
  $zipPath = Join-Path $tmp $assetName
  Write-Host "Downloading $assetName..."
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $zipPath

  Write-Host 'Extracting...'
  Expand-Archive -Path $zipPath -DestinationPath $tmp -Force

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item (Join-Path $tmp 'usageguard.exe') (Join-Path $InstallDir 'usageguard.exe') -Force
  Copy-Item (Join-Path $tmp 'usageguard-desktop.exe') (Join-Path $InstallDir 'usageguard-desktop.exe') -Force

  Write-Host "Installed to $InstallDir"

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  if (-not $userPath.Split(';') -contains $InstallDir) {
    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $InstallDir to user PATH. Restart terminal to use commands globally."
  }

  Write-Host ''
  Write-Host 'Try:'
  Write-Host '  usageguard.exe demo'
  Write-Host '  usageguard-desktop.exe'
}
finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
