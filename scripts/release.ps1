<#
.SYNOPSIS
    Publica una release nueva de Git Agent Desktop.

.DESCRIPTION
    Hace todo el ciclo de una version distribuible:

      1. Sube el numero de version en los cuatro archivos que lo contienen
         (Cargo.toml, tauri.conf.json, package.json, installer\installer.iss).
      2. Compila y verifica el instalador (delegando en scripts\build.ps1).
      3. Hace commit del cambio de version.
      4. Crea el tag y lo empuja a origin.

    El workflow de GitHub Actions (.github\workflows\release.yml) se dispara con
    el tag: ejecuta las pruebas, compila, genera el instalador y el manifiesto
    `latest.json`, y publica ambos en el release. Las apps instaladas lo
    detectan al arrancar y se actualizan solas.

.PARAMETER Version
    Version nueva, en formato x.y.z. Debe ser mayor que la actual.

.PARAMETER Message
    Mensaje del commit de release. Por defecto: "chore(release): vX.Y.Z".

.PARAMETER SkipBuild
    No compila en local (el CI compilara igualmente). Util si solo quieres
    etiquetar.

.PARAMETER DryRun
    Muestra lo que haria sin modificar archivos ni empujar nada.

.EXAMPLE
    pwsh -File scripts\release.ps1 -Version 2.3.0

.EXAMPLE
    pwsh -File scripts\release.ps1 -Version 2.2.2 -SkipBuild -DryRun
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [string]$Message = "",
    [switch]$SkipBuild,
    [switch]$DryRun
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "La version debe tener el formato x.y.z (recibido: '$Version')"
}

$cargo = Get-Content "src-tauri\Cargo.toml" -Raw
if ($cargo -notmatch '(?m)^version\s*=\s*"([^"]+)"') {
    throw "No se pudo leer la version actual de src-tauri\Cargo.toml"
}
$current = $Matches[1]

if ($current -eq $Version) {
    throw "La version $Version ya es la actual. Nada que publicar."
}

$git = Get-Command git -ErrorAction SilentlyContinue
if (-not $git) {
    $candidate = "C:\Program Files\Git\cmd\git.exe"
    if (Test-Path $candidate) { $git = Get-Item $candidate } else { throw "No se encontro git en el PATH" }
}

$tag = "v$Version"
$defaultMessage = "chore(release): v$Version"
$commitMessage = if ([string]::IsNullOrWhiteSpace($Message)) { $defaultMessage } else { $Message }

Write-Host ""
Write-Host "Git Agent Desktop - publicar release" -ForegroundColor Cyan
Write-Host "  version actual : $current"
Write-Host "  version nueva  : $Version"
Write-Host "  tag            : $tag"
Write-Host "  commit         : $commitMessage"
Write-Host ""

if ($DryRun) {
    Write-Host "[DryRun] Se actualizarian estos archivos:" -ForegroundColor Yellow
    Write-Host "  src-tauri\Cargo.toml"
    Write-Host "  src-tauri\tauri.conf.json"
    Write-Host "  package.json"
    Write-Host "  installer\installer.iss"
    if (-not $SkipBuild) { Write-Host "  (se compilaria con scripts\build.ps1)" }
    Write-Host "  git commit + git tag $tag + git push origin main $tag"
    return
}

# --------------------------------------------------------------------------
# 1) Subir la version en los cuatro archivos
# --------------------------------------------------------------------------
$targets = @(
    @{ Path = "src-tauri\Cargo.toml"; Pattern = '(?m)^version\s*=\s*"[^"]+"'; Replacement = "version = `"$Version`"" },
    @{ Path = "src-tauri\tauri.conf.json"; Pattern = '"version":\s*"[^"]+"'; Replacement = "`"version`": `"$Version`"" },
    @{ Path = "package.json"; Pattern = '"version":\s*"[^"]+"'; Replacement = "`"version`": `"$Version`"" },
    @{ Path = "installer\installer.iss"; Pattern = '#define MyAppVersion "[^"]+"'; Replacement = "#define MyAppVersion `"$Version`"" }
)

foreach ($target in $targets) {
    $text = Get-Content $target.Path -Raw
    $updated = [regex]::Replace($text, $target.Pattern, $target.Replacement, 1)
    if ($updated -eq $text) {
        throw "No se pudo actualizar la version en $($target.Path)"
    }
    Set-Content -Path $target.Path -Value $updated -NoNewline
    Write-Host "  version -> $($target.Path)" -ForegroundColor Green
}

# La guia de usuario tambien lleva la version en el encabezado
if (Test-Path "LEEME.txt") {
    $leeme = Get-Content "LEEME.txt" -Raw
    $leeme = $leeme -replace 'GIT AGENT DESKTOP \d+\.\d+\.\d+ - LEEME', "GIT AGENT DESKTOP $Version - LEEME"
    $leeme = $leeme -replace '(?m)^ Git Agent Desktop \d+\.\d+\.\d+$', " Git Agent Desktop $Version"
    Set-Content "LEEME.txt" -Value $leeme -NoNewline
    Write-Host "  version -> LEEME.txt" -ForegroundColor Green
}

# --------------------------------------------------------------------------
# 2) Compilar y verificar el instalador
# --------------------------------------------------------------------------
if (-not $SkipBuild) {
    Write-Host ""
    Write-Host "Compilando (scripts\build.ps1)..." -ForegroundColor Cyan
    & pwsh -File "scripts\build.ps1"
    if ($LASTEXITCODE -ne 0) { throw "La compilacion fallo" }
}

# --------------------------------------------------------------------------
# 3) Commit del cambio de version
# --------------------------------------------------------------------------
& $git.Source add -A
if ($LASTEXITCODE -ne 0) { throw "git add fallo" }

& $git.Source commit -m $commitMessage
if ($LASTEXITCODE -ne 0) {
    Write-Host "  aviso: no habia cambios que commitear" -ForegroundColor Yellow
}

# --------------------------------------------------------------------------
# 4) Tag y push
# --------------------------------------------------------------------------
& $git.Source tag $tag
if ($LASTEXITCODE -ne 0) { throw "No se pudo crear el tag $tag" }

& $git.Source push origin main
if ($LASTEXITCODE -ne 0) { throw "git push de main fallo" }

& $git.Source push origin $tag
if ($LASTEXITCODE -ne 0) { throw "git push del tag fallo" }

Write-Host ""
Write-Host "Release $tag publicada (push del tag)." -ForegroundColor Green
Write-Host ""
Write-Host "GitHub Actions compilara y publicara ahora el instalador y latest.json." -ForegroundColor Yellow
Write-Host "Sigue el progreso con:   gh run watch"
Write-Host "Canal de actualizaciones:"
Write-Host "  https://github.com/<usuario>/<repo>/releases/latest/download/latest.json"
Write-Host ""
Write-Host "Las apps instaladas lo detectaran al arrancar (o con Ajustes > Buscar actualizaciones)."
