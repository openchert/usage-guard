$ErrorActionPreference = 'Stop'

$Repo = 'openchert/usage-guard'
$ApiUrl = "https://api.github.com/repos/$Repo/releases/latest"
$DefaultInstallRoot = Join-Path $env:LOCALAPPDATA 'Programs\usageguard'
$InstallDir = if ($env:INSTALL_DIR) { $env:INSTALL_DIR } else { Join-Path $DefaultInstallRoot 'bin' }
$DesktopExePath = Join-Path $InstallDir 'usageguard-desktop.exe'
$CliExePath = Join-Path $InstallDir 'usageguard.exe'
$StartMenuShortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\UsageGuard.lnk'
$RunKeyPath = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$RunValueName = 'UsageGuard'
$WasAlreadyInstalled = Test-Path $DesktopExePath
$ExistingAutostartValue = (Get-ItemProperty -Path $RunKeyPath -Name $RunValueName -ErrorAction SilentlyContinue).$RunValueName
$HadAutostart = -not [string]::IsNullOrWhiteSpace($ExistingAutostartValue)

$arch = if ([Environment]::Is64BitOperatingSystem) { 'x64' } else { 'x86' }
if ($arch -ne 'x64') {
  throw "Unsupported architecture: $arch. Available release asset: windows-x64"
}

$assetName = 'usage-guard-windows-x64.zip'

Write-Host 'Installing prebuilt UsageGuard binaries for Windows x64. Rust is not required.'
Write-Host 'Fetching latest release metadata...'
$release = Invoke-RestMethod -Uri $ApiUrl
$InstalledVersion = if ([string]::IsNullOrWhiteSpace($release.tag_name)) { 'unknown-version' } else { $release.tag_name }
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
  Copy-Item (Join-Path $tmp 'usageguard.exe') $CliExePath -Force
  Copy-Item (Join-Path $tmp 'usageguard-desktop.exe') $DesktopExePath -Force

  Write-Host "Installed UsageGuard $InstalledVersion to $InstallDir"

  $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
  $pathEntries = if ([string]::IsNullOrWhiteSpace($userPath)) { @() } else { $userPath.Split(';') }
  if (-not ($pathEntries -contains $InstallDir)) {
    $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) { $InstallDir } else { "$userPath;$InstallDir" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "Added $InstallDir to user PATH. Restart terminal to use commands globally."
  }

  try {
    $shell = New-Object -ComObject WScript.Shell
    $shortcut = $shell.CreateShortcut($StartMenuShortcutPath)
    $shortcut.TargetPath = $DesktopExePath
    $shortcut.WorkingDirectory = $InstallDir
    $shortcut.IconLocation = $DesktopExePath
    $shortcut.Description = 'UsageGuard desktop widget'
    $shortcut.Save()
    Write-Host 'Added a Start Menu shortcut so UsageGuard appears in Windows Search.'
  }
  catch {
    Write-Warning "Could not create the Start Menu shortcut: $($_.Exception.Message)"
  }

  try {
    $startupCommand = '"' + $DesktopExePath + '"'
    if ($HadAutostart -or -not $WasAlreadyInstalled) {
      New-Item -Path $RunKeyPath -Force | Out-Null
      New-ItemProperty -Path $RunKeyPath -Name $RunValueName -PropertyType String -Value $startupCommand -Force | Out-Null
      if ($HadAutostart) {
        Write-Host 'Updated the existing Start with Windows entry.'
      }
      else {
        Write-Host 'Enabled Start with Windows for this user. You can turn it off later from the app menu.'
      }
    }
    else {
      Write-Host 'Start with Windows remains disabled on this existing install.'
    }
  }
  catch {
    Write-Warning "Could not update the Start with Windows setting: $($_.Exception.Message)"
  }

  try {
    $running = Get-Process -Name 'usageguard-desktop' -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $running) {
      Start-Process -FilePath $DesktopExePath -WorkingDirectory $InstallDir | Out-Null
      Write-Host 'Launched UsageGuard.'
    }
    else {
      Write-Host 'UsageGuard is already running; skipped auto-launch.'
    }
  }
  catch {
    Write-Warning "Could not launch UsageGuard automatically: $($_.Exception.Message)"
  }

  Write-Host ''
  Write-Host "Installed version: $InstalledVersion"
  Write-Host 'Try:'
  Write-Host '  usageguard demo'
  Write-Host '  usageguard-desktop'
}
finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
