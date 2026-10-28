# Compila o TrayTemp e gera o instalador em dist\TrayTemp-Setup-<versão>.exe
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$PawnIOUrl = 'https://github.com/namazso/PawnIO.Setup/releases/download/2.2.0/PawnIO_setup.exe'
$PawnIOHash = '1F519A22E47187F70A1379A48CA604981C4FCF694F4E65B734AAA74A9FBA3032'

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build falhou' }
$target = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
$exe = Join-Path $target 'release\TrayTemp.exe'

# Instalador do PawnIO, baixado uma vez e conferido pelo SHA-256 publicado no winget.
$deps = Join-Path $PSScriptRoot 'installer\deps'
New-Item -ItemType Directory -Force $deps | Out-Null
$pawnio = Join-Path $deps 'PawnIO_setup.exe'
if (-not (Test-Path $pawnio) -or (Get-FileHash $pawnio -Algorithm SHA256).Hash -ne $PawnIOHash) {
    Write-Host 'Baixando PawnIO_setup.exe...'
    Invoke-WebRequest $PawnIOUrl -OutFile $pawnio
    if ((Get-FileHash $pawnio -Algorithm SHA256).Hash -ne $PawnIOHash) {
        Remove-Item $pawnio
        throw 'SHA-256 do PawnIO_setup.exe não confere'
    }
}

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw 'Inno Setup 6 não encontrado (winget install JRSoftware.InnoSetup)' }

& $iscc /Q "/DExePath=$exe" "/DPawnIOSetup=$pawnio" 'installer\TrayTemp.iss'
if ($LASTEXITCODE -ne 0) { throw 'ISCC falhou' }
Get-ChildItem (Join-Path $PSScriptRoot 'dist') -Filter 'TrayTemp-Setup-*.exe' | Select-Object Name, Length
