<#
.SYNOPSIS
    Publica una actualizacion en el canal local que consulta la aplicacion.

.DESCRIPTION
    Toma el instalador recien compilado (dist-installer\GitAgentSetup.exe), calcula
    su SHA-256 y escribe el manifiesto latest.json en la carpeta del canal:

        %APPDATA%\GitAgentDesktop\updates\latest.json
        %APPDATA%\GitAgentDesktop\updates\GitAgentSetup.exe

    La app instalada detecta la version nueva al arrancar (o desde Ajustes) y se
    actualiza sola: descarga, verifica la firma, aplica los cambios y se reinicia.

.PARAMETER ChannelDir
    Carpeta del canal. Por defecto el canal local que lee la app.

.PARAMETER Notes
    Notas de la version que se muestran en la ventana de actualizacion.

.PARAMETER Mandatory
    Marca la actualizacion como obligatoria.

.PARAMETER NoBuild
    No recompila; usa el instalador existente en dist-installer.

.EXAMPLE
    pwsh -File scripts\publish-update.ps1 -Notes "Corrige el proveedor de IA"

.EXAMPLE
    pwsh -File scripts\publish-update.ps1 -ChannelDir D:\releases -NoBuild
#>
[CmdletBinding()]
param(
    [string]$ChannelDir = (Join-Path $env:APPDATA "GitAgentDesktop\updates"),
    [string]$Notes = "",
    [switch]$Mandatory,
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$cargo = Get-Content "src-tauri\Cargo.toml" -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    throw "No se pudo leer la version de src-tauri\Cargo.toml"
}
$version = $Matches[1]
Write-Host "Version a publicar: $version" -ForegroundColor Cyan

if (-not $NoBuild) {
    Write-Host "Compilando instalador..." -ForegroundColor Cyan
    & pwsh -File "scripts\build.ps1"
    if ($LASTEXITCODE -ne 0) { throw "La compilacion fallo" }
}

$installer = "dist-installer\GitAgentSetup.exe"
if (-not (Test-Path $installer)) {
    throw "No existe $installer. Compila primero con scripts\build.ps1"
}

New-Item -ItemType Directory -Force -Path $ChannelDir | Out-Null

$hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLower()
$size = (Get-Item $installer).Length
Copy-Item $installer (Join-Path $ChannelDir "GitAgentSetup.exe") -Force
if (Test-Path "LEEME.txt") {
    Copy-Item "LEEME.txt" (Join-Path $ChannelDir "LEEME.txt") -Force
}

$manifest = [ordered]@{
    version   = $version
    notes     = $Notes
    url       = "GitAgentSetup.exe"
    sha256    = $hash
    size      = $size
    mandatory = [bool]$Mandatory
}

$manifestPath = Join-Path $ChannelDir "latest.json"
$manifest | ConvertTo-Json -Depth 4 | Set-Content -Path $manifestPath -Encoding utf8

Write-Host ""
Write-Host "Actualizacion publicada" -ForegroundColor Green
Write-Host "  version : $version"
Write-Host "  canal   : $ChannelDir"
Write-Host "  sha256  : $hash"
Write-Host "  tamano  : $([math]::Round($size / 1MB, 2)) MB"
Write-Host "  manifiesto: $manifestPath"
Write-Host ""
Write-Host "La app instalada la detectara al arrancar o con Ajustes > Buscar actualizaciones." -ForegroundColor Yellow
