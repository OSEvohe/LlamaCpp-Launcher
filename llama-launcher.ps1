param(
    [switch]$Status
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$StateDir = Join-Path $ScriptDir ".launcher"
$PidFile = Join-Path $StateDir "llama-server.pid"
$LogFile = Join-Path $StateDir "llama-server.log"
$ErrFile = Join-Path $StateDir "llama-server.err.log"

function Ensure-StateDir {
    if (-not (Test-Path -LiteralPath $StateDir)) {
        New-Item -ItemType Directory -Path $StateDir | Out-Null
    }
}

function Read-Default([string]$Label, [string]$DefaultValue) {
    $value = Read-Host "$Label [$DefaultValue]"
    if ([string]::IsNullOrWhiteSpace($value)) { return $DefaultValue }
    return $value.Trim()
}

function Read-YesNo([string]$Label, [bool]$DefaultValue) {
    $hint = if ($DefaultValue) { "Y/n" } else { "y/N" }
    $value = Read-Host "$Label ($hint)"
    if ([string]::IsNullOrWhiteSpace($value)) { return $DefaultValue }
    switch ($value.Trim().ToLowerInvariant()) {
        "y" { return $true }
        "yes" { return $true }
        "n" { return $false }
        "no" { return $false }
        default { return $DefaultValue }
    }
}

function Test-ProcessAlive([int]$Pid) {
    try {
        $p = Get-Process -Id $Pid -ErrorAction Stop
        return $null -ne $p
    } catch {
        return $false
    }
}

function Get-RunningPid {
    if (-not (Test-Path -LiteralPath $PidFile)) { return $null }
    $raw = Get-Content -LiteralPath $PidFile -ErrorAction SilentlyContinue | Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($raw)) { return $null }
    $pidValue = 0
    if (-not [int]::TryParse($raw.Trim(), [ref]$pidValue)) { return $null }
    if (Test-ProcessAlive -Pid $pidValue) { return $pidValue }
    return $null
}

function Show-Status {
    Ensure-StateDir
    $runningPid = Get-RunningPid
    if ($null -eq $runningPid) {
        Write-Host "llama-server: STOPPED"
    } else {
        Write-Host "llama-server: RUNNING (PID $runningPid)"
    }

    Write-Host "Logs:"
    Write-Host "  stdout: $LogFile"
    Write-Host "  stderr: $ErrFile"
}

function Stop-ExistingIfNeeded {
    $runningPid = Get-RunningPid
    if ($null -eq $runningPid) { return }

    Write-Host "Un llama-server tourne deja (PID $runningPid)."
    $stopNow = Read-YesNo -Label "Le stopper avant de relancer ?" -DefaultValue $true
    if (-not $stopNow) {
        throw "Operation annulee."
    }

    Stop-Process -Id $runningPid -Force
    Remove-Item -LiteralPath $PidFile -ErrorAction SilentlyContinue
    Write-Host "Ancien processus arrete."
}

function Resolve-LlamaServerExe([string]$UserInput) {
    if (-not [string]::IsNullOrWhiteSpace($UserInput)) {
        if (-not (Test-Path -LiteralPath $UserInput)) {
            throw "Chemin invalide: $UserInput"
        }
        return (Resolve-Path -LiteralPath $UserInput).Path
    }

    $cmd = Get-Command llama-server -ErrorAction SilentlyContinue
    if ($null -eq $cmd) {
        throw "llama-server introuvable. Renseigne son chemin complet."
    }

    return $cmd.Source
}

function Start-LlamaServer {
    Ensure-StateDir
    Stop-ExistingIfNeeded

    Write-Host "=== Llama.cpp launcher (terminal) ==="
    Write-Host ""

    $binInput = Read-Host "Chemin de llama-server (laisser vide si dans PATH)"
    $bin = Resolve-LlamaServerExe -UserInput $binInput

    $model = Read-Default -Label "Modele (GGUF) --model" -DefaultValue "C:\\models\\model.gguf"
    if (-not (Test-Path -LiteralPath $model)) {
        throw "Modele introuvable: $model"
    }

    $host = Read-Default -Label "Host --host" -DefaultValue "127.0.0.1"
    $port = Read-Default -Label "Port --port" -DefaultValue "8080"
    $ctx = Read-Default -Label "Contexte --ctx-size" -DefaultValue "4096"
    $threads = Read-Default -Label "Threads --threads" -DefaultValue "8"
    $gpuLayers = Read-Default -Label "GPU layers --n-gpu-layers" -DefaultValue "0"
    $temp = Read-Default -Label "Temperature --temp" -DefaultValue "0.7"
    $topP = Read-Default -Label "Top-p --top-p" -DefaultValue "0.95"
    $batch = Read-Default -Label "Batch --batch-size" -DefaultValue "512"
    $embeddings = Read-YesNo -Label "Activer embeddings (--embeddings)" -DefaultValue $false
    $flashAttn = Read-YesNo -Label "Activer flash attention (--flash-attn)" -DefaultValue $false

    $extra = Read-Host "Options supplementaires (ex: --mirostat 2) ou vide"

    $argList = @(
        "--model", $model
        "--host", $host
        "--port", $port
        "--ctx-size", $ctx
        "--threads", $threads
        "--n-gpu-layers", $gpuLayers
        "--temp", $temp
        "--top-p", $topP
        "--batch-size", $batch
    )

    if ($embeddings) { $argList += "--embeddings" }
    if ($flashAttn) { $argList += "--flash-attn" }
    if (-not [string]::IsNullOrWhiteSpace($extra)) {
        $argList += $extra.Trim().Split(" ", [System.StringSplitOptions]::RemoveEmptyEntries)
    }

    Write-Host ""
    Write-Host "Commande:"
    Write-Host "$bin $($argList -join ' ')"
    Write-Host ""

    $confirm = Read-YesNo -Label "Lancer llama-server en background ?" -DefaultValue $true
    if (-not $confirm) {
        Write-Host "Annule."
        return
    }

    if (Test-Path -LiteralPath $LogFile) { Remove-Item -LiteralPath $LogFile -Force }
    if (Test-Path -LiteralPath $ErrFile) { Remove-Item -LiteralPath $ErrFile -Force }

    $p = Start-Process -FilePath $bin -ArgumentList $argList -RedirectStandardOutput $LogFile -RedirectStandardError $ErrFile -PassThru -WindowStyle Hidden
    $p.Id | Set-Content -LiteralPath $PidFile

    Write-Host ""
    Write-Host "llama-server lance en background."
    Write-Host "PID: $($p.Id)"
    Write-Host "URL: http://$host`:$port"
    Write-Host "stdout: $LogFile"
    Write-Host "stderr: $ErrFile"
}

if ($Status) {
    Show-Status
    exit 0
}

Start-LlamaServer
