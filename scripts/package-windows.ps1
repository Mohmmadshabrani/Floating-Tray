# Package the compiled app and test the install and uninstall on the Windows runner.
param(
    [Parameter(Mandatory)] [string] $SourceDirectory
)

$ErrorActionPreference = 'Stop'
$source = (Resolve-Path $SourceDirectory).Path
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
$payload = Join-Path $dist 'payload'
New-Item -ItemType Directory -Path $payload -Force | Out-Null

foreach ($file in 'target/release/floating-tray.exe', 'hook/YASBTrayHook.dll', 'WINDOWS.md', 'LICENSE') {
    Copy-Item (Join-Path $source $file) -Destination $payload
}
Copy-Item (Join-Path $source 'hook/LICENSE') -Destination (Join-Path $payload 'HOOK-LICENSE.txt')
Compress-Archive -Path (Join-Path $payload '*') -DestinationPath (Join-Path $dist 'Floating-Tray-Windows-x64.zip') -Force

$metadata = cargo metadata --locked --no-deps --format-version 1 --manifest-path (Join-Path $source 'Cargo.toml')
if ($LASTEXITCODE -ne 0) {
    throw 'Could not determine the package version.'
}
$version = ($metadata | ConvertFrom-Json).packages | Where-Object name -eq 'floating-tray' | Select-Object -ExpandProperty version
if (!$version) {
    throw 'Floating Tray version not found.'
}

$compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (!$compiler) {
    $compiler = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
}
if (!(Test-Path -LiteralPath $compiler -PathType Leaf)) {
    throw 'Install Inno Setup 6.3 or newer to build the setup wizard.'
}

& $compiler "/DAppVersion=$version" "/DPayloadDirectory=$payload" "/DOutputDirectory=$dist" (Join-Path $root 'installer/FloatingTray.iss')
if ($LASTEXITCODE -ne 0) {
    throw "Inno Setup failed with exit code $LASTEXITCODE."
}

# Keep the package script usable for local builds and perform the smoke test in CI.
if ($env:GITHUB_ACTIONS -eq 'true') {
    $installed = Join-Path $env:RUNNER_TEMP 'floating-tray-install-test'
    $setup = Join-Path $dist 'Floating-Tray-Setup-x64.exe'
    $flags = '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART'
    $process = Start-Process -FilePath $setup -ArgumentList "$flags /SP- /DIR=`"$installed`"" -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        throw "Setup failed with exit code $($process.ExitCode)."
    }
    foreach ($name in 'floating-tray.exe', 'YASBTrayHook.dll') {
        $expected = (Get-FileHash -LiteralPath (Join-Path $payload $name)).Hash
        $actual = (Get-FileHash -LiteralPath (Join-Path $installed $name)).Hash
        if ($actual -ne $expected) {
            throw "Installed file differs from the packaged file: $name"
        }
    }
    $shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Floating Tray.lnk'
    if (!(Test-Path -LiteralPath $shortcut)) {
        throw 'Setup did not create the Start menu shortcut.'
    }

    $uninstaller = Get-ChildItem -LiteralPath $installed -Filter 'unins*.exe' -File
    $process = Start-Process -FilePath $uninstaller.FullName -ArgumentList $flags -Wait -PassThru
    if ($process.ExitCode -ne 0 -or (Test-Path -LiteralPath $shortcut) -or (Test-Path -LiteralPath (Join-Path $installed 'floating-tray.exe'))) {
        throw "Uninstall smoke test failed with exit code $($process.ExitCode)."
    }
}
