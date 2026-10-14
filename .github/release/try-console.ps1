# Windows' console twin as a person types it (decision 0063, amended 2026-10-14 (Release-prep);
# decision 0064). FOLDER is laid out as a download is: gigasculpt.exe, which opens no console,
# gigasculpt.com, the same program built for the console, and gs.exe. `gigasculpt --help`, and
# `gigasculpt --version` when VERSION is given, typed in PowerShell and in cmd, in that folder and
# from another with the folder on PATH, must reach gigasculpt.com and print there; and
# `gigasculpt serve --headless` typed in cmd keeps its terminal until the session stops. CI runs
# it on a build laid out so, the release workflow on the unpacked download.
#
#   pwsh .github/release/try-console.ps1 FOLDER [VERSION]
param([Parameter(Mandatory)] [string] $Folder, [string] $Version)
$ErrorActionPreference = 'Stop'
$Folder = (Resolve-Path $Folder).Path
Set-Location $Folder

# The Subsystem field of a program's PE header (0x5C past the header, whose offset is at 0x3C):
# 2 opens no console, 3 runs in the one that started it.
function Subsystem($file) {
  $b = [IO.File]::ReadAllBytes((Resolve-Path $file))
  [BitConverter]::ToUInt16($b, [BitConverter]::ToInt32($b, 0x3C) + 0x5C)
}
if ((Subsystem gigasculpt.exe) -ne 2) { throw "gigasculpt.exe would open a console" }
if ((Subsystem gigasculpt.com) -ne 3) { throw "gigasculpt.com would not print in a terminal" }

# What each question must print.
$asks = [ordered]@{ '--help' = 'Usage: gigasculpt' }
if ($Version) { $asks['--version'] = "^gigasculpt $([regex]::Escape($Version))\s*$" }

# `gigasculpt` typed in PowerShell (as $typed) and in cmd, from where they are now.
function Typed($how, $typed) {
  $found = (Get-Command $typed | Select-Object -First 1).Source
  if ($found -notlike '*\gigasculpt.com') { throw "PowerShell ($how) runs $found for $typed" }
  $first = (cmd /c 'where gigasculpt') | Select-Object -First 1
  if ($first -notlike '*\gigasculpt.com') { throw "cmd ($how) runs $first for gigasculpt" }
  foreach ($ask in $asks.Keys) {
    $said = & $typed $ask | Out-String
    if ($LASTEXITCODE -or $said -notmatch $asks[$ask]) { throw "PowerShell ($how), $typed ${ask}: $said" }
    $said = cmd /c "gigasculpt $ask" | Out-String
    if ($LASTEXITCODE -or $said -notmatch $asks[$ask]) { throw "cmd ($how), gigasculpt ${ask}: $said" }
  }
  Write-Host "${how}: PowerShell's $typed and cmd's gigasculpt run gigasculpt.com and print there ($($asks.Keys -join ', '))"
}
# PowerShell runs a program in the folder it is in only by its path; cmd finds it there itself.
Typed 'in its folder' '.\gigasculpt'
$env:PATH = "$Folder;$env:PATH"
Push-Location (New-Item -ItemType Directory -Force (Join-Path ([IO.Path]::GetTempPath()) 'try-console-elsewhere'))
Typed 'elsewhere, the folder on PATH' 'gigasculpt'
Pop-Location

# serve --headless keeps its terminal until the session stops.
$serve = Start-Process cmd -NoNewWindow -PassThru -ArgumentList '/c',
  'gigasculpt serve --headless --session console --stop-key console-key'
$null = $serve.Handle
$up = $false
for ($i = 0; $i -lt 180 -and -not $up -and -not $serve.HasExited; $i++) {
  .\gs -s console query mesh.hash *> $null
  $up = $LASTEXITCODE -eq 0
  if (-not $up) { Start-Sleep 1 }
}
if (-not $up -or $serve.HasExited) { throw "the session never answered, or its terminal stopped waiting" }
.\gs -s console stop --key console-key
if (-not $serve.WaitForExit(60000)) { throw "the session did not stop" }
if ($serve.ExitCode -ne 0) { throw "gigasculpt serve --headless ended with $($serve.ExitCode)" }
Write-Host "cmd: gigasculpt serve --headless kept its terminal until the session stopped"
