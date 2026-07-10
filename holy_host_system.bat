@echo off
cd /d "%~dp0"
chcp 65001 >nul
title Прокидываю хосты

NET SESSION >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo Запрос на получение административных прав...
    powershell -Command "Start-Process -FilePath '%~dpnx0' -Verb RunAs"
    exit /b
)

powershell -NoProfile -ExecutionPolicy Bypass -Command "$c = Get-Content '%~f0'; $i = [Array]::IndexOf($c, '#=== PURE POWERSHELL ===#'); Invoke-Expression ($c[($i+1)..$c.Length] -join [Environment]::NewLine)"
exit /b

#=== PURE POWERSHELL ===#
$ListsDir = Join-Path (Get-Location) "lists"
if (-not (Test-Path $ListsDir)) { New-Item -ItemType Directory -Path $ListsDir | Out-Null }
$HostsList = Join-Path $ListsDir "hosts-list.txt"
$HostsFile = "C:\Windows\System32\drivers\etc\hosts"
$GithubUrl = "https://raw.githubusercontent.com/HolyLightRU/HolyZapret/main/lists/hosts-list.txt"

Write-Host 'Проверяю обновления hosts-list...' -ForegroundColor Yellow
try {
    Invoke-WebRequest -Uri $GithubUrl -OutFile $HostsList -ErrorAction Stop
    Write-Host 'Файл hosts-list успешно обновлён' -ForegroundColor Green
} catch {
    Write-Host "Ошибка при загрузке файла: $_" -ForegroundColor Red
}

Write-Host 'Обновляю записи в файле hosts...' -ForegroundColor Yellow
if (Test-Path $HostsFile) {
    Copy-Item -Path $HostsFile -Destination "$HostsFile.bak" -Force
}

if (-not (Test-Path $HostsList)) {
    Write-Host "Критическая ошибка: Файл списка хостов не найден!" -ForegroundColor Red
    Exit
}

$newMappings = [System.Collections.Generic.Dictionary[string, string]]::new([System.StringComparer]::OrdinalIgnoreCase)
foreach ($line in Get-Content $HostsList) {
    if ($line -match '^\s*([^\s]+)\s+([^\s]+)') {
        $newMappings[$Matches[2]] = $Matches[1]
    }
}

# Читаем старый hosts
$oldHostsContent = if (Test-Path $HostsFile) { Get-Content $HostsFile } else { @() }
$updatedHosts = [System.Collections.Generic.List[string]]::new()
$seenDomains = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)
$oldDomains = [System.Collections.Generic.HashSet[string]]::new([System.StringComparer]::OrdinalIgnoreCase)

foreach ($line in $oldHostsContent) {
    if ($line -match '^\s*#' -or $line -match '^\s*$') {
        $updatedHosts.Add($line)
        continue
    }

    if ($line -match '^\s*([^\s]+)\s+([^\s]+)') {
        $domain = $Matches[2]
        $oldDomains.Add($domain) | Out-Null

        # Если домен есть в новом списке или уже обрабатывался (дубликат) — пропускаем старую запись
        if ($newMappings.ContainsKey($domain) -or $seenDomains.Contains($domain)) {
            continue
        }
        $seenDomains.Add($domain) | Out-Null
        $updatedHosts.Add($line)
    } else {
        $updatedHosts.Add($line)
    }
}

foreach ($kvp in $newMappings.GetEnumerator()) {
    $domain = $kvp.Key
    $ip = $kvp.Value
    if ($oldDomains.Contains($domain)) {
        Write-Host "Обновляю: $domain -> $ip" -ForegroundColor Cyan
    } else {
        Write-Host "Добавлено: $ip $domain" -ForegroundColor Magenta
    }
    $updatedHosts.Add("$ip $domain")
}

# Запись без BOM-маркеров, чтобы системный DNS-клиент Windows не спотыкался
[System.IO.File]::WriteAllLines($HostsFile, $updatedHosts)
Write-Host 'Готово!' -ForegroundColor Green