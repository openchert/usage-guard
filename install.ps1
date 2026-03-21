$ErrorActionPreference = 'Stop'

$Repo = 'openchert/usage-guard'
$ApiUrl = "https://api.github.com/repos/$Repo/releases/latest"
$DefaultCliInstallRoot = Join-Path $env:LOCALAPPDATA 'Programs\usageguard'
$CliInstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $DefaultCliInstallRoot 'bin' }
$CliExePath = Join-Path $CliInstallDir 'usageguard.exe'
$CliAssetName = 'usage-guard-windows-cli-x64.zip'

function Find-ReleaseAsset($release, [scriptblock]$predicate) {
  return $release.assets | Where-Object $predicate | Select-Object -First 1
}

function Find-InstalledDesktopExe {
  $candidates = @(
    (Join-Path $env:LOCALAPPDATA 'Programs\UsageGuard\usageguard-desktop.exe'),
    (Join-Path $env:LOCALAPPDATA 'UsageGuard\usageguard-desktop.exe')
  )

  foreach ($path in $candidates) {
    if (Test-Path $path) {
      return $path
    }
  }

  foreach ($root in @(
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*',
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*'
  )) {
    $entry = Get-ItemProperty -Path $root -ErrorAction SilentlyContinue |
      Where-Object { $_.DisplayName -eq 'UsageGuard' } |
      Select-Object -First 1

    if (-not $entry) {
      continue
    }

    if (-not [string]::IsNullOrWhiteSpace($entry.InstallLocation)) {
      $path = Join-Path $entry.InstallLocation 'usageguard-desktop.exe'
      if (Test-Path $path) {
        return $path
      }
    }

    if (-not [string]::IsNullOrWhiteSpace($entry.DisplayIcon) -and (Test-Path $entry.DisplayIcon)) {
      return $entry.DisplayIcon
    }
  }

  return $null
}

$arch = if ([Environment]::Is64BitOperatingSystem) { 'x64' } else { 'x86' }
if ($arch -ne 'x64') {
  throw "Unsupported architecture: $arch. Available release asset: windows-x64"
}

Write-Host 'Installing UsageGuard for Windows x64. Rust is not required.'
Write-Host 'Fetching latest release metadata...'
$release = Invoke-RestMethod -Uri $ApiUrl
$installedVersion = if ([string]::IsNullOrWhiteSpace($release.tag_name)) { 'unknown-version' } else { $release.tag_name }

$setupAsset = Find-ReleaseAsset $release { $_.name -match '_x64-setup\.exe$' }
if (-not $setupAsset) {
  throw "Could not find a Windows setup executable in the latest release. Check https://github.com/$Repo/releases"
}

$cliAsset = Find-ReleaseAsset $release { $_.name -eq $CliAssetName }

$tmp = Join-Path $env:TEMP ("usageguard-" + [Guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $tmp | Out-Null

try {
  $setupPath = Join-Path $tmp $setupAsset.name
  Write-Host "Downloading $($setupAsset.name)..."
  Invoke-WebRequest -Uri $setupAsset.browser_download_url -OutFile $setupPath

  Write-Host 'Launching the UsageGuard installer...'
  $installer = Start-Process -FilePath $setupPath -Wait -PassThru
  if ($installer.ExitCode -ne 0) {
    throw "UsageGuard installer failed with exit code $($installer.ExitCode)"
  }

  Write-Host "Installed UsageGuard $installedVersion using the Windows installer."

  if ($cliAsset) {
    $cliZipPath = Join-Path $tmp $CliAssetName
    $cliExtractDir = Join-Path $tmp 'cli'

    Write-Host "Downloading $CliAssetName..."
    Invoke-WebRequest -Uri $cliAsset.browser_download_url -OutFile $cliZipPath

    Write-Host 'Installing CLI...'
    Expand-Archive -Path $cliZipPath -DestinationPath $cliExtractDir -Force
    New-Item -ItemType Directory -Path $CliInstallDir -Force | Out-Null
    Copy-Item (Join-Path $cliExtractDir 'usageguard.exe') $CliExePath -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    $pathEntries = if ([string]::IsNullOrWhiteSpace($userPath)) { @() } else { $userPath.Split(';') }
    if (-not ($pathEntries -contains $CliInstallDir)) {
      $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $CliInstallDir } else { "$userPath;$CliInstallDir" }
      [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
      Write-Host "Added $CliInstallDir to user PATH. Restart terminal to use commands globally."
    }
  }
  else {
    Write-Warning "Could not find $CliAssetName in the latest release. Skipping CLI installation."
  }

  try {
    $desktopExePath = Find-InstalledDesktopExe
    $running = Get-Process -Name 'usageguard-desktop' -ErrorAction SilentlyContinue | Select-Object -First 1

    if (-not $running -and $desktopExePath) {
      Start-Process -FilePath $desktopExePath | Out-Null
      Write-Host 'Launched UsageGuard once to complete first-run setup.'
    }
    elseif (-not $desktopExePath) {
      Write-Host 'Launch UsageGuard once from the Start Menu to complete first-run setup.'
    }
  }
  catch {
    Write-Warning "Could not launch UsageGuard automatically: $($_.Exception.Message)"
  }

  Write-Host ''
  Write-Host "Installed version: $installedVersion"
  if (Test-Path $CliExePath) {
    Write-Host 'Try:'
    Write-Host '  usageguard status'
  }
  Write-Host 'Open UsageGuard from the Start Menu if it did not launch automatically.'
}
finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
