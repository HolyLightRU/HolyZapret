$hasErrors = $false

$rootDir = Split-Path $PSScriptRoot
$listsDir = Join-Path $rootDir "lists"
$expListsDir = Join-Path $rootDir "exp-list"
$utilsDir = Join-Path $rootDir "utils"
$resultsDir = Join-Path $utilsDir "результаты тестов"
if (-not (Test-Path $resultsDir)) { New-Item -ItemType Directory -Path $resultsDir | Out-Null }

function New-OrderedDict { New-Object System.Collections.Specialized.OrderedDictionary }
function Add-OrSet {
    param($dict, $key, $val)
    if ($dict.Contains($key)) { $dict[$key] = $val } else { $dict.Add($key, $val) }
}

function Convert-Target {
    param(
        [string]$Name,
        [string]$Value
    )

    if ($Value -like "PING:*") {
        $ping = $Value -replace '^PING:\s*', ''
        $url = $null
        $pingTarget = $ping
    } else {
        $url = $Value
        $pingTarget = $url -replace "^https?://", "" -replace "/.*$", ""
    }

    return (New-Object PSObject -Property @{
        Name       = $Name
        Url        = $url
        PingTarget = $pingTarget
    })
}

function Get-DpiSuite {
    return @(
        @{ Id = "US.CF-01"; Provider = "Cloudflare"; Url = "https://cdn.cookielaw.org/scripttemplates/202501.2.0/otBannerSdk.js"; Times = 1 }
        @{ Id = "US.CF-02"; Provider = "Cloudflare"; Url = "https://genshin.jmp.blue/characters/all#"; Times = 1 }
        @{ Id = "US.CF-03"; Provider = "Cloudflare"; Url = "https://api.frankfurter.dev/v1/2000-01-01..2002-12-31"; Times = 1 }
        @{ Id = "US.DO-01"; Provider = "DigitalOcean"; Url = "https://genderize.io/"; Times = 2 }
        @{ Id = "DE.HE-01"; Provider = "Hetzner"; Url = "https://j.dejure.org/jcg/doctrine/doctrine_banner.webp"; Times = 1 }
        @{ Id = "FI.HE-01"; Provider = "Hetzner"; Url = "https://tcp1620-01.dubybot.live/1MB.bin"; Times = 1 }
        @{ Id = "FI.HE-02"; Provider = "Hetzner"; Url = "https://tcp1620-02.dubybot.live/1MB.bin"; Times = 1 }
        @{ Id = "FI.HE-03"; Provider = "Hetzner"; Url = "https://tcp1620-05.dubybot.live/1MB.bin"; Times = 1 }
        @{ Id = "FI.HE-04"; Provider = "Hetzner"; Url = "https://tcp1620-06.dubybot.live/1MB.bin"; Times = 1 }
        @{ Id = "FR.OVH-01"; Provider = "OVH"; Url = "https://eu.api.ovh.com/console/rapidoc-min.js"; Times = 1 }
        @{ Id = "FR.OVH-02"; Provider = "OVH"; Url = "https://ovh.sfx.ovh/10M.bin"; Times = 1 }
        @{ Id = "SE.OR-01"; Provider = "Oracle"; Url = "https://oracle.sfx.ovh/10M.bin"; Times = 1 }
        @{ Id = "DE.AWS-01"; Provider = "AWS"; Url = "https://tms.delta.com/delta/dl_anderson/Bootstrap.js"; Times = 1 }
        @{ Id = "US.AWS-01"; Provider = "AWS"; Url = "https://corp.kaltura.com/wp-content/cache/min/1/wp-content/themes/airfleet/dist/styles/theme.css"; Times = 1 }
        @{ Id = "US.GC-01"; Provider = "Google Cloud"; Url = "https://api.usercentrics.eu/gvl/v3/en.json"; Times = 1 }
        @{ Id = "US.FST-01"; Provider = "Fastly"; Url = "https://openoffice.apache.org/images/blog/rejected.png"; Times = 1 }
        @{ Id = "US.FST-02"; Provider = "Fastly"; Url = "https://www.juniper.net/etc.clientlibs/juniper/clientlibs/clientlib-site/resources/fonts/lato/Lato-Regular.woff2"; Times = 1 }
        @{ Id = "PL.AKM-01"; Provider = "Akamai"; Url = "https://www.lg.com/lg5-common-gp/library/jquery.min.js"; Times = 1 }
        @{ Id = "PL.AKM-02"; Provider = "Akamai"; Url = "https://media-assets.stryker.com/is/image/stryker/gateway_1?$max_width_1410$"; Times = 1 }
        @{ Id = "US.CDN77-01"; Provider = "CDN77"; Url = "https://cdn.eso.org/images/banner1920/eso2520a.jpg"; Times = 1 }
        @{ Id = "DE.CNTB-01"; Provider = "Contabo"; Url = "https://cloudlets.io/wp-content/themes/Avada/includes/lib/assets/fonts/fontawesome/webfonts/fa-solid-900.woff2"; Times = 1 }
        @{ Id = "FR.SW-01"; Provider = "Scaleway"; Url = "https://renklisigorta.com.tr/teklif-al"; Times = 1 }
        @{ Id = "US.CNST-01"; Provider = "Constant"; Url = "https://cdn.xuansiwei.com/common/lib/font-awesome/4.7.0/fontawesome-webfont.woff2?v=4.7.0"; Times = 1 }
    )
}

function Build-DpiTargets {
    param([string]$CustomUrl)
    $suite = Get-DpiSuite
    $targets = @()
    if ($CustomUrl) {
        $targets += @{ Id = "CUSTOM"; Provider = "Custom"; Url = $CustomUrl }
    } else {
        foreach ($entry in $suite) {
            $repeat = if ($entry.Times) { $entry.Times } else { 1 }
            for ($i = 0; $i -lt $repeat; $i++) {
                $suffix = if ($repeat -gt 1) { "@$i" } else { "" }
                $targets += @{ Id = "$($entry.Id)$suffix"; Provider = $entry.Provider; Url = $entry.Url }
            }
        }
    }
    return $targets
}

function Invoke-DpiSuite {
    param(
        [array]$Targets,
        [int]$TimeoutSeconds,
        [int]$RangeBytes,
        [int]$WarnMinKB,
        [int]$WarnMaxKB,
        [int]$MaxParallel
    )

    $tests = @(
        @{ Label = "HTTP";   Args = @("--http1.1") },
        @{ Label = "TLS1.2"; Args = @("--tlsv1.2", "--tls-max", "1.2") },
        @{ Label = "TLS1.3"; Args = @("--tlsv1.3", "--tls-max", "1.3") }
    )

    $rangeSpec = "0-$($RangeBytes - 1)"
    $warnDetected = $false

    Write-Host "[ИНФО] Цели: $($Targets.Count). Диапазон: $rangeSpec байт; Таймаут: $TimeoutSeconds с" -ForegroundColor Magenta
    Write-Host "[ИНФО] Запуск проверок DPI TCP 16-20 (параллельно: $MaxParallel)..." -ForegroundColor DarkMagenta

    $runspacePool = [runspacefactory]::CreateRunspacePool(1, $MaxParallel)
    $runspacePool.Open()

    $scriptBlock = {
        param($target, $tests, $rangeSpec, $TimeoutSeconds, $WarnMinKB, $WarnMaxKB)

        $warned = $false
        $lines = @()

        foreach ($test in $tests) {
            $curlArgs = @("-L", "--range", $rangeSpec, "-m", $TimeoutSeconds, "-w", "%{http_code} %{size_download}", "-o", "NUL", "-s") + $test.Args + $target.Url

            $output = & curl.exe @curlArgs 2>&1
            $exit = $LASTEXITCODE
            $text = ($output | Out-String).Trim()

            $code = "NA"
            $sizeBytes = 0

            if ($text -match '^(?<code>\d{3})\s+(?<size>\d+)$') {
                $code = $matches['code']
                $sizeBytes = [int64]$matches['size']
            } elseif ($text -match 'not supported|does not support') {
                $code = "UNSUP"
            } elseif ($text) {
                $code = "ERR"
            }

            $sizeKB = [math]::Round($sizeBytes / 1024, 1)
            $status = "OK"
            $color = "Green"

            if ($code -eq "UNSUP") {
                $status = "НЕ ПОДДЕРЖИВАЕТСЯ"
                $color = "Yellow"
            } elseif ($exit -ne 0 -or $code -eq "ERR" -or $code -eq "NA") {
                $status = "ОШИБКА"
                $color = "Red"
            }

            if (($sizeKB -ge $WarnMinKB) -and ($sizeKB -le $WarnMaxKB) -and ($exit -ne 0)) {
                $status = "ВЕРОЯТНО ЗАБЛОКИРОВАНО"
                $color = "Yellow"
                $warned = $true
            }

            $lines += [PSCustomObject]@{
                TargetId   = $target.Id
                Provider   = $target.Provider
                TestLabel  = $test.Label
                Code       = $code
                SizeBytes  = $sizeBytes
                SizeKB     = $sizeKB
                Status     = $status
                Color      = $color
                Warned     = $warned
            }
        }

        return [PSCustomObject]@{
            TargetId = $target.Id
            Provider = $target.Provider
            Lines    = $lines
            Warned   = $warned
        }
    }

    $runspaces = @()
    foreach ($target in $Targets) {
        $ps = [powershell]::Create().AddScript($scriptBlock)
        $ps.AddArgument($target) > $null
        $ps.AddArgument($tests) > $null
        $ps.AddArgument($rangeSpec) > $null
        $ps.AddArgument($TimeoutSeconds) > $null
        $ps.AddArgument($WarnMinKB) > $null
        $ps.AddArgument($WarnMaxKB) > $null
        $ps.RunspacePool = $runspacePool
        $runspaces += [PSCustomObject]@{ Powershell = $ps; Handle = $ps.BeginInvoke() }
    }

    $results = @()
    foreach ($rs in $runspaces) {
        $results += $rs.Powershell.EndInvoke($rs.Handle)
        $rs.Powershell.Dispose()
    }
    $runspacePool.Close()
    $runspacePool.Dispose()

    foreach ($res in $results) {
        Write-Host "`n=== $($res.TargetId) [$($res.Provider)] ===" -ForegroundColor DarkMagenta
        foreach ($line in $res.Lines) {
            $msg = "[{0}][{1}] код={2} размер={3} байт ({4} КБ) статус={5}" -f $line.TargetId, $line.TestLabel, $line.Code, $line.SizeBytes, $line.SizeKB, $line.Status
            Write-Host $msg -ForegroundColor $line.Color
            if ($line.Status -eq "ВЕРОЯТНО ЗАБЛОКИРОВАНО") {
                Write-Host "  Шаблон заморозки 16-20 КБ — цензор блокирует эту стратегию." -ForegroundColor Yellow
            }
        }
        if (-not $res.Warned) {
            Write-Host "  Заморозка 16-20 КБ не обнаружена." -ForegroundColor Green
        } else {
            $warnDetected = $true
        }
    }

    if ($warnDetected) {
        Write-Host "`n[ПРЕДУПРЕЖДЕНИЕ] Обнаружена блокировка DPI TCP 16-20. Попробуйте другую стратегию." -ForegroundColor Red
    } else {
        Write-Host "`n[OK] Блокировка DPI TCP 16-20 не обнаружена." -ForegroundColor Green
    }

    return $results
}

# === Проверки окружения ===
$currentPrincipal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $currentPrincipal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Host "[ОШИБКА] Запустите скрипт от имени Администратора" -ForegroundColor Red
    $hasErrors = $true
} else {
    Write-Host "[OK] Права администратора есть" -ForegroundColor Green
}

if (-not (Get-Command "curl.exe" -ErrorAction SilentlyContinue)) {
    Write-Host "[ОШИБКА] curl.exe не найден в PATH" -ForegroundColor Red
    $hasErrors = $true
} else {
    Write-Host "[OK] curl.exe найден" -ForegroundColor Green
}

if (Get-Service -Name "zapret" -ErrorAction SilentlyContinue) {
    Write-Host "[ОШИБКА] Сервис zapret установлен. Удалите его через manager.bat перед тестами." -ForegroundColor Red
    $hasErrors = $true
}

if ($hasErrors) {
    Write-Host "`nИсправьте ошибки и перезапустите скрипт." -ForegroundColor Yellow
    Read-Host "Нажмите Enter для выхода"
    exit 1
}

# === Настройки ===
$dpiTimeoutSeconds = 5
$dpiRangeBytes = 262144
$dpiWarnMinKB = 14
$dpiWarnMaxKB = 22
$dpiMaxParallel = 8
$dpiCustomUrl = $env:MONITOR_URL
if ($env:MONITOR_TIMEOUT) { $dpiTimeoutSeconds = [int]$env:MONITOR_TIMEOUT }
if ($env:MONITOR_RANGE) { $dpiRangeBytes = [int]$env:MONITOR_RANGE }
if ($env:MONITOR_WARN_MINKB) { $dpiWarnMinKB = [int]$env:MONITOR_WARN_MINKB }
if ($env:MONITOR_WARN_MAXKB) { $dpiWarnMaxKB = [int]$env:MONITOR_WARN_MAXKB }
if ($env:MONITOR_MAX_PARALLEL) { $dpiMaxParallel = [int]$env:MONITOR_MAX_PARALLEL }
$dpiTargets = Build-DpiTargets -CustomUrl $dpiCustomUrl

# === Сбор всех .bat стратегий (как в manager.bat) ===
$batFiles = @()
# Корень
Get-ChildItem -Path $rootDir -Filter "*.bat" | Where-Object {
    $n = $_.Name.ToLower()
    $n -notlike "service*" -and $n -ne "holy_host_system.bat" -and $n -ne "manager.bat"
} | ForEach-Object { $batFiles += $_ }

# Подпапки
Get-ChildItem -Path $rootDir -Directory | ForEach-Object {
    Get-ChildItem -Path $_.FullName -Filter "*.bat" | Where-Object {
        $n = $_.Name.ToLower()
        $n -notlike "service*" -and $n -ne "holy_host_system.bat"
    } | ForEach-Object { $batFiles += $_ }
}
$batFiles = $batFiles | Sort-Object FullName

if ($batFiles.Count -eq 0) {
    Write-Host "[ОШИБКА] Не найдено ни одной стратегии (*.bat)" -ForegroundColor Red
    Read-Host "Нажмите Enter для выхода"
    exit 1
}

# === Вспомогательные функции для HolyZapret ===
function Stop-HolyZapret { Get-Process -Name "HolyZapret" -ErrorAction SilentlyContinue | Stop-Process -Force }

function Get-HolyZapretSnapshot {
    Get-CimInstance Win32_Process -Filter "Name='HolyZapret.exe'" -ErrorAction SilentlyContinue |
        Select-Object ProcessId, CommandLine, ExecutablePath
}

function Restore-HolyZapretSnapshot {
    param($snapshot)
    if (-not $snapshot) { return }
    $current = (Get-HolyZapretSnapshot).CommandLine
    foreach ($p in $snapshot) {
        if ($p.CommandLine -and $current -notcontains $p.CommandLine) {
            Start-Process -FilePath $p.ExecutablePath -ArgumentList ($p.CommandLine -replace "^`"[^`"]+`"", "").Trim() -WorkingDirectory (Split-Path $p.ExecutablePath) -WindowStyle Minimized
        }
    }
}

$originalHolyZapret = Get-HolyZapretSnapshot

# === Выбор типа и режима теста ===
function Read-TestType {
    while ($true) {
        Write-Host "`nВыберите тип теста:" -ForegroundColor DarkMagenta
        Write-Host "  [1] Стандартные тесты (HTTP + ping)" -ForegroundColor Magenta
        Write-Host "  [2] DPI-проверки (заморозка TCP 16-20)" -ForegroundColor Magenta
        $c = Read-Host "Введите 1 или 2"
        if ($c -eq "1") { return "standard" }
        if ($c -eq "2") { return "dpi" }
        Write-Host "Неверный выбор." -ForegroundColor Yellow
    }
}

function Read-ModeSelection {
    while ($true) {
        Write-Host "`nРежим запуска:" -ForegroundColor DarkMagenta
        Write-Host "  [1] Все конфиги" -ForegroundColor Magenta
        Write-Host "  [2] Выбрать вручную" -ForegroundColor Magenta
        Write-Host "  [3] Оптимальные (из optimal_strategies.txt)" -ForegroundColor Magenta
        $c = Read-Host "Введите 1, 2 или 3"
        if ($c -eq "1") { return "all" }
        if ($c -eq "2") { return "select" }
        if ($c -eq "3") { return "optimal" }
        Write-Host "Неверный выбор." -ForegroundColor Yellow
    }
}

function Read-ConfigSelection {
    param($all)
    while ($true) {
        Write-Host "`nДоступные конфиги:" -ForegroundColor DarkMagenta
        for ($i = 0; $i -lt $all.Count; $i++) {
            Write-Host "  [$($i+1)] $($all[$i].FullName -replace [regex]::Escape($rootDir+'\'), '')" -ForegroundColor Magenta
        }
        $input = Read-Host "Номера через запятую (пример: 1,3,5) или 0 для всех"
        if ($input -eq "0") { return $all }
        $nums = ($input -split '[,\s]+' | Where-Object { $_ -match '^\d+$' } | ForEach-Object { [int]$_ - 1 }) | Where-Object { $_ -ge 0 -and $_ -lt $all.Count } | Sort-Object -Unique
        if ($nums.Count -gt 0) { return $nums | ForEach-Object { $all[$_] } }
        Write-Host "Ничего не выбрано." -ForegroundColor Yellow
    }
}

$globalResults = @()
while ($true) {
    $testType = Read-TestType
    $mode = Read-ModeSelection

    $selectedBats = $batFiles

    if ($mode -eq "select") {
        $selected = Read-ConfigSelection -all $batFiles
        $selectedBats = $selected
    } elseif ($mode -eq "optimal") {
        $optimalFile = Join-Path $utilsDir "optimal_strategies.txt"
        if (-not (Test-Path $optimalFile)) {
            Write-Host "[ОШИБКА] Файл optimal_strategies.txt не найден в utils!" -ForegroundColor Red
            continue
        }
        $optimalPaths = Get-Content $optimalFile | Where-Object { $_.Trim() -ne '' } | ForEach-Object { $_.Trim() }
        $optimalBats = @()
        foreach ($p in $optimalPaths) {
            $fullPath = Join-Path $rootDir $p
            if (Test-Path $fullPath -PathType Leaf) {
                $optimalBats += Get-Item $fullPath
            } else {
                Write-Host "[ПРЕДУПРЕЖДЕНИЕ] Путь '$p' не найден или не .bat файл." -ForegroundColor Yellow
            }
        }
        if ($optimalBats.Count -eq 0) {
            Write-Host "[ОШИБКА] Нет валидных стратегий в optimal_strategies.txt" -ForegroundColor Red
            continue
        }
        $selectedBats = $optimalBats
    }

    # === Загрузка целей для стандартного теста ===
    $targetList = @()
    $maxNameLen = 10
    if ($testType -eq "standard") {
        $targetsFile = Join-Path $utilsDir "targets.txt"
        $raw = New-OrderedDict
        if (Test-Path $targetsFile) {
            Get-Content $targetsFile | Where-Object { $_ -match '^\s*(\w+)\s*=\s*"(.+)"' } | ForEach-Object {
                Add-OrSet $raw $matches[1] $matches[2]
            }
        }
        if ($raw.Count -eq 0) {
            Write-Host "[ИНФО] targets.txt не найден или пуст — используются стандартные цели" -ForegroundColor Magenta
            $default = @{
                "Discord Main"           = "https://discord.com"
                "Discord Gateway"        = "https://gateway.discord.gg"
                "Discord CDN"            = "https://cdn.discordapp.com"
                "YouTube Web"            = "https://www.youtube.com"
                "YouTube Video"          = "https://redirector.googlevideo.com"
                "Google"                 = "https://www.google.com"
                "Cloudflare"             = "https://www.cloudflare.com"
                "CF DNS 1.1.1.1"         = "PING:1.1.1.1"
                "Google DNS 8.8.8.8"     = "PING:8.8.8.8"
            }
            $default.Keys | ForEach-Object { Add-OrSet $raw $_ $default[$_] }
        }

        foreach ($k in $raw.Keys) { $targetList += Convert-Target -Name $k -Value $raw[$k] }
        $maxNameLen = ($targetList.Name | Measure-Object -Maximum Length).Maximum
        if ($maxNameLen -lt 10) { $maxNameLen = 10 }
    }

    Write-Host "`n[ПРЕДУПРЕЖДЕНИЕ] Тестирование может занять несколько минут..." -ForegroundColor Yellow

    Write-Host "`n============================================================" -ForegroundColor DarkMagenta
    Write-Host "                ТЕСТЫ СТРАТЕГИЙ HOLYZAPRET" -ForegroundColor DarkMagenta
    Write-Host "                Режим: $testType    Конфигов: $($selectedBats.Count)" -ForegroundColor DarkMagenta
    Write-Host "============================================================" -ForegroundColor DarkMagenta

    try {

        $num = 0
        foreach ($file in $selectedBats) {
            $num++
            $relPath = $file.FullName -replace [regex]::Escape($rootDir+'\'), ''
            Write-Host "`n------------------------------------------------------------" -ForegroundColor DarkMagenta
            Write-Host "  [$num/$($selectedBats.Count)] $relPath" -ForegroundColor Magenta
            Write-Host "------------------------------------------------------------" -ForegroundColor DarkMagenta

            Stop-HolyZapret
            $proc = Start-Process "cmd.exe" -ArgumentList "/c `"$($file.FullName)`"" -WorkingDirectory $rootDir -PassThru -WindowStyle Minimized
            Start-Sleep -Seconds 5

            if ($testType -eq "standard") {
                $results = @()
                $pool = [runspacefactory]::CreateRunspacePool(1, 8)
                $pool.Open()
                $jobs = @()

                foreach ($t in $targetList) {
                    $ps = [powershell]::Create().AddScript({
                        param($target, $timeout)
                        $http = @()
                        if ($target.Url) {
                            $tests = @("HTTP", "TLS1.2", "TLS1.3")
                            foreach ($ver in $tests) {
                                $args = @("-I", "-s", "-m", $timeout, "-o", "NUL", "-w", "%{http_code}")
                                if ($ver -ne "HTTP") { $args += "--tlsv$($ver -replace 'TLS','')"; if ($ver -eq "TLS1.2") { $args += "--tls-max","1.2" } }
                                $out = & curl.exe @args $target.Url 2>&1
                                if ($out -match "not supported") { $http += "$ver`:НЕ ПОДДЕРЖИВАЕТСЯ" }
                                elseif ($LASTEXITCODE -eq 0) { $http += "$ver`:OK" } else { $http += "$ver`:ОШИБКА" }
                            }
                        }
                        $ping = "н/д"
                        if ($target.PingTarget) {
                            $p = Test-Connection -ComputerName $target.PingTarget -Count 3 -ErrorAction SilentlyContinue
                            if ($p) { $ping = "{0:N0} мс" -f ($p.ResponseTime | Measure-Object -Average).Average } else { $ping = "Таймаут" }
                        }
                        [PSCustomObject]@{ Name = $target.Name; Http = $http -join " "; Ping = $ping; IsUrl = [bool]$target.Url }
                    }).AddArgument($t).AddArgument(5)
                    $ps.RunspacePool = $pool
                    $jobs += @{ Ps = $ps; Handle = $ps.BeginInvoke() }
                }

                foreach ($j in $jobs) {
                    $results += $j.Ps.EndInvoke($j.Handle)
                    $j.Ps.Dispose()
                }
                $pool.Close(); $pool.Dispose()

                foreach ($r in $results) {
                    Write-Host "  $($r.Name.PadRight($maxNameLen)) " -NoNewline
                    if ($r.IsUrl) {
                        $r.Http -split " " | ForEach-Object {
                            $c = if ($_ -like "*OK*") { "Green" } elseif ($_ -like "*НЕ ПОДДЕРЖИВАЕТСЯ*") { "Yellow" } else { "Red" }
                            Write-Host $_ -NoNewline -ForegroundColor $c; Write-Host " " -NoNewline
                        }
                        Write-Host "| Пинг: $($r.Ping)" -ForegroundColor Magenta
                    } else {
                        Write-Host "Пинг: $($r.Ping)" -ForegroundColor Magenta
                    }
                }

                $globalResults += @{ Config = $relPath; Type = "standard"; Results = $results }
            } else {
                Write-Host "  > Запуск DPI-проверок..." -ForegroundColor DarkMagenta
                $dpiRes = Invoke-DpiSuite -Targets $dpiTargets -TimeoutSeconds $dpiTimeoutSeconds -RangeBytes $dpiRangeBytes -WarnMinKB $dpiWarnMinKB -WarnMaxKB $dpiWarnMaxKB -MaxParallel $dpiMaxParallel
                $globalResults += @{ Config = $relPath; Type = "dpi"; Results = $dpiRes }
            }

            Stop-HolyZapret
            if ($proc -and -not $proc.HasExited) { Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue }
        }

        # === Аналитика и сохранение ===
        $analytics = @{}
        foreach ($gr in $globalResults) {
            $cfg = $gr.Config
            if (-not $analytics[$cfg]) { $analytics[$cfg] = @{ OK = 0; ERROR = 0; UNSUP = 0; BLOCKED = 0; PingOK = 0; PingFail = 0 } }
            if ($gr.Type -eq "standard") {
                foreach ($r in $gr.Results) {
                    if ($r.IsUrl) {
                        ($r.Http -split " ") | ForEach-Object {
                            if ($_ -like "*OK*") { $analytics[$cfg].OK++ }
                            elseif ($_ -like "*НЕ ПОДДЕРЖИВАЕТСЯ*") { $analytics[$cfg].UNSUP++ }
                            else { $analytics[$cfg].ERROR++ }
                        }
                    }
                    if ($r.Ping -ne "Таймаут" -and $r.Ping -ne "н/д") { $analytics[$cfg].PingOK++ } else { $analytics[$cfg].PingFail++ }
                }
            } else {
                foreach ($tr in $gr.Results) {
                    foreach ($line in $tr.Lines) {
                        switch ($line.Status) {
                            "OK"                  { $analytics[$cfg].OK++ }
                            "ОШИБКА"              { $analytics[$cfg].ERROR++ }
                            "НЕ ПОДДЕРЖИВАЕТСЯ"   { $analytics[$cfg].UNSUP++ }
                            "ВЕРОЯТНО ЗАБЛОКИРОВАНО" { $analytics[$cfg].BLOCKED++ }
                        }
                    }
                }
            }
        }

        Write-Host "`n=== АНАЛИТИКА ===" -ForegroundColor DarkMagenta
        $best = $null; $max = -1
        foreach ($cfg in $analytics.Keys) {
            $a = $analytics[$cfg]
            $score = $a.OK
            if ($score -gt $max) { $max = $score; $best = $cfg }
            if ($a.PingOK -or $a.PingFail) {
                Write-Host "$cfg → OK: $($a.OK) | Ошибок: $($a.ERROR) | Неподдерживается: $($a.UNSUP) | Пинг OK: $($a.PingOK)/Ошибка: $($a.PingFail)" -ForegroundColor Magenta
            } else {
                Write-Host "$cfg → OK: $($a.OK) | Ошибок: $($a.ERROR) | Неподдерживается: $($a.UNSUP) | Заблокировано: $($a.BLOCKED)" -ForegroundColor Magenta
            }
        }

        Write-Host "`nЛучшая стратегия: $best" -ForegroundColor Green

        $date = Get-Date -Format "yyyy-MM-dd_HH-mm-ss"
        $outFile = Join-Path $resultsDir "result_$date.txt"
        $globalResults | ForEach-Object {
            "Конфиг: $($_.Config) (Тип: $($_.Type))" | Out-File $outFile -Append -Encoding UTF8
            # детализация опущена для краткости, но можно добавить
        }
        "Лучшая стратегия: $best" | Out-File $outFile -Append -Encoding UTF8
        Write-Host "`nРезультаты сохранены: $outFile" -ForegroundColor Green

    } finally {
        Stop-HolyZapret
        Restore-HolyZapretSnapshot -snapshot $originalHolyZapret
    }

    Write-Host "`nНажмите любую клавишу для возврата в меню..." -ForegroundColor Yellow
    [void][System.Console]::ReadKey($true)
}