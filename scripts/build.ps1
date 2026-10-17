<#
.SYNOPSIS
    Compila Git Agent Desktop de principio a fin (hoja de ruta, Seccion 13).

.DESCRIPTION
    Ejecuta los tres pasos secuenciales del plan:
      1. npm install && npm run build           (interfaz TypeScript + CSS)
      2. cargo build --release                  (binario nativo en Rust)
      3. iscc installer\installer.iss           (instalador bootstrapper)

.PARAMETER SkipFrontend
    Omite la compilacion del frontend (reutiliza dist/ existente).

.PARAMETER SkipRust
    Omite la compilacion de Rust (reutiliza target\release).

.PARAMETER SkipInstaller
    Omite el empaquetado con Inno Setup.

.EXAMPLE
    pwsh -File scripts\build.ps1

.EXAMPLE
    pwsh -File scripts\build.ps1 -SkipFrontend
#>
[CmdletBinding()]
param(
    [switch]$SkipFrontend,
    [switch]$SkipRust,
    [switch]$SkipInstaller
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Write-Step([string]$Text) {
    Write-Host ""
    Write-Host "==============================================================" -ForegroundColor DarkCyan
    Write-Host "  $Text" -ForegroundColor Cyan
    Write-Host "==============================================================" -ForegroundColor DarkCyan
}

function Resolve-Tool([string]$Name, [string[]]$Candidates) {
    $command = Get-Command $Name -ErrorAction SilentlyContinue
    if ($command) { return $command.Source }
    foreach ($candidate in $Candidates) {
        if ($candidate -and (Test-Path $candidate)) { return $candidate }
    }
    throw "No se encontro '$Name'. Instalalo o agregalo al PATH."
}

$started = Get-Date
Write-Host "Git Agent Desktop - build completo" -ForegroundColor Green
Write-Host "Raiz: $root"

# --------------------------------------------------------------------------
Write-Step "Paso 1/3 - Interfaz web (TypeScript + CSS)"
if (-not $SkipFrontend) {
    $node = Resolve-Tool "node" @("C:\Program Files\nodejs\node.exe")
    # Se usa npm.cmd (no npm.ps1): al invocarlo con un interprete explicito, el
    # script de PowerShell se intentaria ejecutar con Node y fallaria.
    $npm = Join-Path (Split-Path $node) "npm.cmd"
    if (-not (Test-Path $npm)) {
        $npm = Resolve-Tool "npm.cmd" @("$env:APPDATA\npm\npm.cmd")
    }
    Write-Host "node: $node"
    Write-Host "npm : $npm"

    if (-not (Test-Path "node_modules")) {
        Write-Host "Instalando dependencias (npm install)..."
        & $npm install --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw "npm install fallo con codigo $LASTEXITCODE" }
    }

    & $npm run build
    if ($LASTEXITCODE -ne 0) { throw "npm run build fallo con codigo $LASTEXITCODE" }

    if (-not (Test-Path "dist\index.html")) { throw "No se genero dist\index.html" }
    Write-Host "Frontend OK: dist\index.html" -ForegroundColor Green
}
else {
    Write-Host "Omitido (-SkipFrontend)." -ForegroundColor Yellow
}

# --------------------------------------------------------------------------
Write-Step "Paso 2/3 - Binario nativo (Rust / Tauri v2)"
if (-not $SkipRust) {
    $cargo = Resolve-Tool "cargo" @("$env:USERPROFILE\.cargo\bin\cargo.exe")
    Write-Host "cargo: $cargo"

    Push-Location "src-tauri"
    try {
        # `custom-protocol` embebe dist/ en el binario: sin ella Tauri compila
        # en modo desarrollo y la ventana intenta cargar devUrl (localhost:1420).
        & $cargo build --release --features custom-protocol
        if ($LASTEXITCODE -ne 0) { throw "cargo build fallo con codigo $LASTEXITCODE" }
    }
    finally {
        Pop-Location
    }

    $exe = "src-tauri\target\release\git-agent-desktop.exe"
    if (-not (Test-Path $exe)) { throw "No se genero $exe" }
    $size = [math]::Round((Get-Item $exe).Length / 1MB, 2)
    Write-Host "Binario OK: $exe ($size MB)" -ForegroundColor Green
}
else {
    Write-Host "Omitido (-SkipRust)." -ForegroundColor Yellow
}

# --------------------------------------------------------------------------
Write-Step "Paso 3/3 - Instalador bootstrapper (Inno Setup)"
if (-not $SkipInstaller) {
    $iscc = Resolve-Tool "iscc" @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    )
    Write-Host "iscc: $iscc"

    & $iscc "installer\installer.iss"
    if ($LASTEXITCODE -ne 0) { throw "ISCC fallo con codigo $LASTEXITCODE" }

    $setup = "dist-installer\GitAgentSetup.exe"
    if (-not (Test-Path $setup)) { throw "No se genero $setup" }

    # Guia para el usuario final junto al instalador (se entrega en la misma carpeta)
    if (Test-Path "LEEME.txt") {
        Copy-Item "LEEME.txt" "dist-installer\LEEME.txt" -Force
        Write-Host "Guia copiada: dist-installer\LEEME.txt"
    }

    $size = [math]::Round((Get-Item $setup).Length / 1MB, 2)
    Write-Host "Instalador OK: $setup ($size MB)" -ForegroundColor Green
}
else {
    Write-Host "Omitido (-SkipInstaller)." -ForegroundColor Yellow
}

# --------------------------------------------------------------------------
$elapsed = (Get-Date) - $started
Write-Host ""
Write-Host "Compilacion finalizada en $([math]::Round($elapsed.TotalSeconds, 1)) s" -ForegroundColor Green
if (Test-Path "dist-installer\GitAgentSetup.exe") {
    Write-Host "Distribuible: $root\dist-installer\GitAgentSetup.exe" -ForegroundColor Green
}
