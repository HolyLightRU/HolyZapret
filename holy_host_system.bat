@echo off
cd /d "%~dp0"
chcp 65001 >nul
title Прокидываю хосты

NET SESSION >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo Запрос на получение административных прав...
    powershell -NoProfile -Command "Start-Process -FilePath '%~dpnx0' -Verb RunAs"
    exit /b
)

powershell -NoProfile -ExecutionPolicy Bypass -Command "$c = [System.IO.File]::ReadAllLines(\"%~f0\", [System.Text.Encoding]::UTF8); $i = [Array]::IndexOf($c, '#=== PURE POWERSHELL ===#'); if ($i -lt 0 -or $i -ge ($c.Length - 1)) { Write-Host 'Не найден блок PowerShell в holy_host_system.bat' -ForegroundColor Red; exit 1 }; Invoke-Expression ($c[($i+1)..($c.Length-1)] -join [Environment]::NewLine)"
exit /b

#=== PURE POWERSHELL ===#
$ErrorActionPreference = 'Stop'

$ListsDir = Join-Path (Get-Location) "lists"
if (-not (Test-Path $ListsDir)) { New-Item -ItemType Directory -Path $ListsDir | Out-Null }
$HostsList = Join-Path $ListsDir "hosts-list.txt"
$HostsFile = "C:\Windows\System32\drivers\etc\hosts"
$GithubUrl = "https://raw.githubusercontent.com/HolyLightRU/HolyZapret/main/lists/hosts-list.txt"
$BlockBegin = '# >>> HolyZapret managed block >>>'
$BlockEnd = '# <<< HolyZapret managed block <<<'
$ProxyIps = @('45.155.204.190', '104.25.158.178', '149.154.167.220', '37.230.192.51')

function Read-HostsList {
    param([string]$Path)

    $map = New-Object -TypeName 'System.Collections.Generic.Dictionary[string,string]' -ArgumentList ([System.StringComparer]::OrdinalIgnoreCase)
    $order = New-Object -TypeName 'System.Collections.Generic.List[string]'
    $conflicts = New-Object -TypeName 'System.Collections.Generic.List[string]'

    if (-not (Test-Path $Path)) {
        return [pscustomobject]@{ Map = $map; Order = $order; Conflicts = $conflicts }
    }

    foreach ($line in [System.IO.File]::ReadAllLines($Path, [System.Text.Encoding]::UTF8)) {
        $text = $line.Trim()
        if ($text.Length -eq 0 -or $text.StartsWith('#')) { continue }

        $hash = $text.IndexOf('#')
        if ($hash -ge 0) { $text = $text.Substring(0, $hash).Trim() }
        if ($text.Length -eq 0) { continue }

        $parts = @($text -split '\s+' | Where-Object { $_.Length -gt 0 })
        if ($parts.Count -lt 2) { continue }

        $ip = $parts[0]
        $parsedIp = $null
        if (-not [System.Net.IPAddress]::TryParse($ip, [ref]$parsedIp)) { continue }

        for ($i = 1; $i -lt $parts.Count; $i++) {
            $domain = $parts[$i].Trim().ToLowerInvariant()
            if ($domain.Length -eq 0) { continue }
            if ($domain -notmatch '^[a-z0-9_][a-z0-9._-]*$') { continue }

            if ($map.ContainsKey($domain)) {
                if ($map[$domain] -ne $ip) {
                    $conflicts.Add("$domain ($($map[$domain]) -> $ip)") | Out-Null
                    $map[$domain] = $ip
                }
            } else {
                $map[$domain] = $ip
                $order.Add($domain) | Out-Null
            }
        }
    }

    return [pscustomobject]@{ Map = $map; Order = $order; Conflicts = $conflicts }
}

function Get-HostsListVersion {
    param([string]$Path)

    if (-not (Test-Path $Path)) { return [version]'0.0.0' }

    foreach ($line in [System.IO.File]::ReadAllLines($Path, [System.Text.Encoding]::UTF8)) {
        if ($line -match '^\s*#\s*HolyZapret\s+hosts-list\s+v?([0-9]+(?:\.[0-9]+){0,3})\s*$') {
            try { return [version]$Matches[1] } catch { return [version]'0.0.0' }
        }
    }

    return [version]'0.0.0'
}

function Read-HostsFile {
    param([string]$Path)

    if (-not (Test-Path $Path)) {
        return [pscustomobject]@{
            Lines    = @()
            Encoding = (New-Object -TypeName System.Text.UTF8Encoding -ArgumentList $false)
        }
    }

    $bytes = [System.IO.File]::ReadAllBytes($Path)
    $offset = 0
    $encoding = $null

    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        $encoding = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList $true
        $offset = 3
    } elseif ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFF -and $bytes[1] -eq 0xFE) {
        $encoding = [System.Text.Encoding]::Unicode
        $offset = 2
    } elseif ($bytes.Length -ge 2 -and $bytes[0] -eq 0xFE -and $bytes[1] -eq 0xFF) {
        $encoding = [System.Text.Encoding]::BigEndianUnicode
        $offset = 2
    } else {
        try {
            $strictUtf8 = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList $false, $true
            $null = $strictUtf8.GetString($bytes)
            $encoding = New-Object -TypeName System.Text.UTF8Encoding -ArgumentList $false
        } catch {
            $encoding = [System.Text.Encoding]::Default
        }
    }

    $text = $encoding.GetString($bytes, $offset, $bytes.Length - $offset)
    $lines = @($text -split "\r?\n")

    return [pscustomobject]@{ Lines = $lines; Encoding = $encoding }
}

function Write-HostsFile {
    param(
        [string]$Path,
        [string[]]$Lines,
        $Encoding
    )

    $text = ($Lines -join "`r`n") + "`r`n"
    [System.IO.File]::WriteAllText($Path, $text, $Encoding)
}

# --- Обновление списка -------------------------------------------------------
Write-Host 'Проверяю обновления hosts-list...' -ForegroundColor Yellow

$remoteTemp = "$HostsList.download"
try {
    if (Test-Path $remoteTemp) { Remove-Item $remoteTemp -Force }
    Invoke-WebRequest -Uri $GithubUrl -OutFile $remoteTemp -UseBasicParsing -TimeoutSec 15 -ErrorAction Stop

    $remoteData = Read-HostsList -Path $remoteTemp
    if ($remoteData.Map.Count -lt 50) {
        throw "получен подозрительно короткий список ($($remoteData.Map.Count) записей)"
    }

    $remoteVersion = Get-HostsListVersion -Path $remoteTemp
    $localVersion = Get-HostsListVersion -Path $HostsList

    if ($remoteVersion -ge $localVersion) {
        Move-Item -Path $remoteTemp -Destination $HostsList -Force
        Write-Host "Список hosts-list обновлён: v$remoteVersion, $($remoteData.Map.Count) доменов" -ForegroundColor Green
    } else {
        Write-Host "Локальный список новее (v$localVersion > v$remoteVersion), скачивание пропущено" -ForegroundColor Yellow
    }
} catch {
    Write-Host "Не удалось скачать hosts-list: $($_.Exception.Message)" -ForegroundColor Yellow
    Write-Host 'Использую локальный список.' -ForegroundColor Yellow
} finally {
    if (Test-Path $remoteTemp) { Remove-Item $remoteTemp -Force -ErrorAction SilentlyContinue }
}

if (-not (Test-Path $HostsList)) {
    Write-Host "Критическая ошибка: файл $HostsList не найден!" -ForegroundColor Red
    exit 1
}

$listData = Read-HostsList -Path $HostsList
if ($listData.Map.Count -eq 0) {
    Write-Host 'Критическая ошибка: список hosts-list пуст или повреждён!' -ForegroundColor Red
    exit 1
}

if ($listData.Conflicts.Count -gt 0) {
    Write-Host 'Внимание: для некоторых доменов указаны разные IP. Использовано последнее значение:' -ForegroundColor Yellow
    $listData.Conflicts | Select-Object -First 20 | ForEach-Object { Write-Host "  $_" -ForegroundColor Yellow }
}

$listVersion = Get-HostsListVersion -Path $HostsList

# --- Обновление hosts --------------------------------------------------------
Write-Host 'Обновляю записи в файле hosts...' -ForegroundColor Yellow

if (Test-Path $HostsFile) {
    Copy-Item -Path $HostsFile -Destination "$HostsFile.bak" -Force
}

$hostsData = Read-HostsFile -Path $HostsFile
$oldLines = @($hostsData.Lines)

$beginCount = 0
$endCount = 0
foreach ($line in $oldLines) {
    if ($line.Trim() -eq $BlockBegin) { $beginCount++ }
    if ($line.Trim() -eq $BlockEnd) { $endCount++ }
}
$hasBlock = ($beginCount -gt 0 -and $endCount -gt 0)

# Удаляем старый управляемый блок (или его осиротевшие маркеры).
$withoutBlock = New-Object -TypeName 'System.Collections.Generic.List[string]'
$insideBlock = $false
foreach ($line in $oldLines) {
    $trim = $line.Trim()
    if ($hasBlock) {
        if ($trim -eq $BlockBegin) { $insideBlock = $true; continue }
        if ($trim -eq $BlockEnd) { $insideBlock = $false; continue }
        if ($insideBlock) { continue }
    } else {
        if ($trim -eq $BlockBegin -or $trim -eq $BlockEnd) { continue }
    }
    $withoutBlock.Add($line) | Out-Null
}

# Убираем старые записи HolyZapret и дубли доменов из нового списка.
$filtered = New-Object -TypeName 'System.Collections.Generic.List[string]'
foreach ($line in $withoutBlock) {
    $trim = $line.Trim()
    if ($trim.Length -eq 0 -or $trim.StartsWith('#')) {
        $filtered.Add($line) | Out-Null
        continue
    }

    $hash = $line.IndexOf('#')
    if ($hash -ge 0) {
        $content = $line.Substring(0, $hash)
        $comment = $line.Substring($hash)
    } else {
        $content = $line
        $comment = ''
    }

    $parts = @($content.Trim() -split '\s+' | Where-Object { $_.Length -gt 0 })
    if ($parts.Count -lt 2) {
        $filtered.Add($line) | Out-Null
        continue
    }

    $ip = $parts[0]
    $keepDomains = New-Object -TypeName 'System.Collections.Generic.List[string]'
    $removedManaged = $false

    for ($i = 1; $i -lt $parts.Count; $i++) {
        $domain = $parts[$i].ToLowerInvariant()
        if ($listData.Map.ContainsKey($domain)) {
            $removedManaged = $true
            continue
        }
        $keepDomains.Add($parts[$i]) | Out-Null
    }

    if ($keepDomains.Count -eq 0) { continue }

    if ($removedManaged) {
        $newLine = "$ip`t$($keepDomains -join ' ')"
        if ($comment.Length -gt 0) { $newLine += " $comment" }
        $filtered.Add($newLine) | Out-Null
    } elseif ($ProxyIps -contains $ip -and $parts.Count -eq 2) {
        # Устаревшая запись HolyZapret, которой больше нет в списке.
        continue
    } else {
        $filtered.Add($line) | Out-Null
    }
}

while ($filtered.Count -gt 0 -and $filtered[$filtered.Count - 1].Trim().Length -eq 0) {
    $filtered.RemoveAt($filtered.Count - 1)
}

$filtered.Add('') | Out-Null
$filtered.Add($BlockBegin) | Out-Null
if ($listVersion -ne [version]'0.0.0') {
    $filtered.Add("# HolyZapret hosts-list v$listVersion") | Out-Null
}
$filtered.Add('# Generated automatically by holy_host_system.bat. Do not edit this block.') | Out-Null
foreach ($domain in $listData.Order) {
    $filtered.Add("$($listData.Map[$domain])`t$domain") | Out-Null
}
$filtered.Add($BlockEnd) | Out-Null

try {
    Write-HostsFile -Path $HostsFile -Lines $filtered.ToArray() -Encoding $hostsData.Encoding

    $checkData = Read-HostsList -Path $HostsFile
    $missing = 0
    foreach ($domain in $listData.Order) {
        if (-not $checkData.Map.ContainsKey($domain) -or $checkData.Map[$domain] -ne $listData.Map[$domain]) {
            $missing++
        }
    }

    if ($missing -gt 0) {
        throw "после записи не найдено $missing доменов"
    }

    Write-Host "Готово! Записей в hosts: $($listData.Map.Count). Резервная копия: ${HostsFile}.bak" -ForegroundColor Green
    ipconfig /flushdns | Out-Null
} catch {
    Write-Host "Ошибка записи hosts: $($_.Exception.Message)" -ForegroundColor Red
    if (Test-Path "$HostsFile.bak") {
        Copy-Item -Path "$HostsFile.bak" -Destination $HostsFile -Force
        Write-Host 'Восстановлена резервная копия hosts.bak' -ForegroundColor Yellow
    }
    exit 1
}
