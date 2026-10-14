# The Microsoft C and C++ runtime a Windows download needs (decision 0064). Windows does not
# come with it: the Visual C++ Redistributable is installed by other programs, and a build
# machine always has it, so a run there cannot show it missing. So each runtime DLL a program in
# FOLDER imports must sit beside it. With -From, the folder of those DLLs in Visual Studio, each
# one missing is copied beside its program first: Microsoft's local deployment. Imports are read
# as third_party/quadwild/build.sh reads them, the DLL names in the program's bytes.
#
#   pwsh .github/release/vc-runtime.ps1 FOLDER [-From DIR]
param([Parameter(Mandatory)] [string] $Folder, [string] $From)
$ErrorActionPreference = 'Stop'
$Folder = (Resolve-Path $Folder).Path

# Each runtime DLL a program under FOLDER imports that is not beside it, as the path it needs.
function Missing {
  foreach ($program in Get-ChildItem -Recurse -File $Folder -Include *.exe, *.com) {
    $bytes = [Text.Encoding]::ASCII.GetString([IO.File]::ReadAllBytes($program.FullName))
    $names = [regex]::Matches($bytes, '(?i)\b(vcruntime|msvcp|vcomp|concrt)[0-9a-z_]*\.dll') |
      ForEach-Object { $_.Value.ToLowerInvariant() } | Sort-Object -Unique
    foreach ($name in $names) {
      Write-Host "$($program.FullName.Substring($Folder.Length + 1)) imports $name"
      $beside = Join-Path $program.DirectoryName $name
      if (-not (Test-Path $beside)) { $beside }
    }
  }
}

if ($From) {
  foreach ($path in Missing | Sort-Object -Unique) {
    Copy-Item (Join-Path $From (Split-Path -Leaf $path)) $path
    Write-Host "copied $(Split-Path -Leaf $path) beside it"
  }
}
$missing = @(Missing | Sort-Object -Unique)
if ($missing) { throw "the download lacks runtime DLLs its programs import:`n$($missing -join "`n")" }
Write-Host "every runtime DLL a program imports is beside it"
