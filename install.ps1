# BatteryWidget installer.
#
#   irm https://raw.githubusercontent.com/hitzjano/BatteryWidget/main/install.ps1 | iex
#
# Downloads the newest release, verifies its SHA-256 checksum, installs it to
# %LOCALAPPDATA%\Programs\BatteryWidget, adds a Start menu shortcut and starts it.
# No admin rights needed. Running it again updates an existing installation.

& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'   # the progress bar makes downloads very slow in PowerShell 5.1
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'hitzjano/BatteryWidget'
    $exeName = 'BatteryWidget.exe'
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\BatteryWidget'
    $target = Join-Path $installDir $exeName

    Write-Host 'BatteryWidget installer' -ForegroundColor Cyan

    # Newest published release, pre-releases included (/releases/latest would skip those).
    $headers = @{ 'User-Agent' = 'BatteryWidget-installer'; 'Accept' = 'application/vnd.github+json' }
    $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$repo/releases?per_page=10" -Headers $headers
    $release = @($releases | Where-Object { -not $_.draft }) | Select-Object -First 1
    if (-not $release) { throw "No release of $repo found." }

    $exeAsset = $release.assets | Where-Object { $_.name -eq $exeName } | Select-Object -First 1
    $shaAsset = $release.assets | Where-Object { $_.name -eq "$exeName.sha256" } | Select-Object -First 1
    if (-not $exeAsset) { throw "Release $($release.tag_name) has no $exeName." }

    Write-Host "Downloading $($release.tag_name)..."
    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("BatteryWidget-" + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $tmp | Out-Null
    try {
        $tmpExe = Join-Path $tmp $exeName
        Invoke-WebRequest -Uri $exeAsset.browser_download_url -OutFile $tmpExe -UseBasicParsing

        if ($shaAsset) {
            $shaFile = Join-Path $tmp "$exeName.sha256"
            Invoke-WebRequest -Uri $shaAsset.browser_download_url -OutFile $shaFile -UseBasicParsing
            $expected = ((Get-Content $shaFile -Raw).Trim() -split '\s+')[0]
            $actual = (Get-FileHash $tmpExe -Algorithm SHA256).Hash
            if ($actual -ne $expected) { throw "Checksum mismatch (expected $expected, got $actual). Aborting." }
            Write-Host 'Checksum OK.'
        } else {
            Write-Warning 'Release has no checksum file; skipping verification.'
        }

        # Stop a running instance so the file can be replaced.
        $running = Get-Process -Name 'BatteryWidget' -ErrorAction SilentlyContinue
        if ($running) {
            Write-Host 'Stopping running BatteryWidget...'
            $running | Stop-Process -Force
            $running | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
        }

        New-Item -ItemType Directory -Path $installDir -Force | Out-Null
        Copy-Item $tmpExe $target -Force
    } finally {
        Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
    }

    # Start menu shortcut.
    $startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
    $shell = New-Object -ComObject WScript.Shell
    $lnk = $shell.CreateShortcut((Join-Path $startMenu 'BatteryWidget.lnk'))
    $lnk.TargetPath = $target
    $lnk.WorkingDirectory = $installDir
    $lnk.Description = 'Battery levels of your wireless devices in the taskbar'
    $lnk.Save()

    Write-Host "Installed to $target" -ForegroundColor Green
    Start-Process -FilePath $target
    Write-Host 'BatteryWidget is running. Right-click the tray icon for settings (e.g. "Start with Windows").'
}
