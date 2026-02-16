@echo off
chcp 65001 > nul
set "LOCAL_VERSION=1.4.2"

:: External commands
if "%~1"=="status_zapret" (
    call :test_service zapret soft
    exit /b
)

if "%~1"=="check_updates" (
    if not "%~2"=="soft" (
        start /b service check_updates soft
    ) else (
        call :service_check_updates soft
    )
    exit /b
)

if "%1"=="admin" (
    echo Started with admin rights
) else (
    echo Requesting admin rights...
    powershell -Command "Start-Process 'cmd.exe' -ArgumentList '/c \"\"%~f0\" admin\"' -Verb RunAs"
    exit /b
)

if "%~1"=="load_game_filter" (
    call :game_switch_status
    exit /b
)

:: ====================================
:: Проверка обновлений — только после получения прав администратора
:: ====================================
if not "%1"=="admin" goto :skip_update_check

powershell -Command "Write-Host 'Проверка наличия обновлений...' -ForegroundColor Cyan"

set "REMOTE_URL=https://raw.githubusercontent.com/HolyLightRU/HolyZapret/main/manager.bat"
set "TEMP_FILE=%TEMP%\holyzapret_remote_version.txt"

powershell -Command "try { $content = (Invoke-WebRequest -Uri '%REMOTE_URL%' -UseBasicParsing -TimeoutSec 10 -ErrorAction Stop).Content; if ($content -match 'set \"LOCAL_VERSION=(.*?)\"') { $matches[1] | Out-File -FilePath '%TEMP_FILE%' -Encoding UTF8 } else { 'ERROR_PARSE' | Out-File -FilePath '%TEMP_FILE%' -Encoding UTF8 } } catch { 'ERROR' | Out-File -FilePath '%TEMP_FILE%' -Encoding UTF8 }"

if not exist "%TEMP_FILE%" (
    powershell -Command "Write-Host 'Не удалось проверить обновления (проблема с загрузкой).' -ForegroundColor Yellow"
    goto :skip_update_check
)

set /p REMOTE_VERSION= < "%TEMP_FILE%"
del "%TEMP_FILE%" 2>nul

if "%REMOTE_VERSION%"=="ERROR" (
    powershell -Command "Write-Host 'Не удалось загрузить файл с GitHub (нет интернета или ошибка сети).' -ForegroundColor Yellow"
	pause
    goto :skip_update_check
)

if "%REMOTE_VERSION%"=="ERROR_PARSE" (
    powershell -Command "Write-Host 'Не удалось распознать версию в файле на GitHub.' -ForegroundColor Yellow"
	pause
    goto :skip_update_check
)

if not "%LOCAL_VERSION%"=="%REMOTE_VERSION%" (
    echo.
    powershell -Command "Write-Host '==================================================================' -ForegroundColor Red"
    powershell -Command "Write-Host 'Обнаружена новая версия HolyZapret!' -ForegroundColor Red"
    powershell -Command "Write-Host 'Ваша версия: %LOCAL_VERSION%    ║    Последняя версия: %REMOTE_VERSION%' -ForegroundColor Red"
    powershell -Command "Write-Host 'Настоятельно рекомендуется обновиться!' -ForegroundColor Red"
    powershell -Command "Write-Host '==================================================================' -ForegroundColor Red"
    echo.
    timeout /t 5
    goto :skip_update_check
) else (
    powershell -Command "Write-Host 'Ваша версия %LOCAL_VERSION% — самая актуальная.' -ForegroundColor Green"
)

:skip_update_check
echo.
:: ====================================
:: MENU ================================
setlocal EnableDelayedExpansion
chcp 65001 > nul
:menu
cls
powershell -Command "Write-Host 'HolyZapret %LOCAL_VERSION% Console' -ForegroundColor DarkMagenta; Write-Host '=======================' -ForegroundColor Magenta; Write-Host '1. Установить сервис (скрытая версия)' -ForegroundColor White; Write-Host '2. Удалить запрет и сервис' -ForegroundColor Cyan; Write-Host '3. Проверить текущий статус Запрета' -ForegroundColor White; Write-Host '4. Запустить диагностику' -ForegroundColor Magenta; Write-Host '5. Обновить hosts файл (holy_host_system)' -ForegroundColor DarkMagenta; Write-Host '6. Обновить auto_update.bat' -ForegroundColor White; Write-Host '7. Проверить доступность сайтов' -ForegroundColor Cyan; Write-Host '8. Проверить стратегии' -ForegroundColor Magenta; Write-Host '0. Выход' -ForegroundColor White"

set "menu_choice="
set /p "menu_choice=Выберите пункт (0-8): "

if "%menu_choice%"=="1" goto service_install
if "%menu_choice%"=="2" goto service_remove
if "%menu_choice%"=="3" goto service_status
if "%menu_choice%"=="4" goto service_diagnostics
if "%menu_choice%"=="5" goto run_hosts_system
if "%menu_choice%"=="6" goto update_auto_update
if "%menu_choice%"=="7" goto check_sites
if "%menu_choice%"=="8" goto run_tests
if "%menu_choice%"=="0" exit /b
goto menu


:: RUN HOLY_HOST_SYSTEM ================
:run_hosts_system
cls
chcp 65001 > nul
powershell -Command "Write-Host 'Запускаю holy_host_system.bat...' -ForegroundColor Cyan"

if exist "%~dp0holy_host_system.bat" (
    call "%~dp0holy_host_system.bat"
) else (
    powershell -Command "Write-Host 'Файл holy_host_system.bat не найден в текущей директории!' -ForegroundColor Red"
)

pause
goto menu

:: RUN TESTS =============================
:run_tests
chcp 65001 >nul
cls

:: Require PowerShell 2.0+
powershell -NoProfile -Command "if ($PSVersionTable -and $PSVersionTable.PSVersion -and $PSVersionTable.PSVersion.Major -ge 2) { exit 0 } else { exit 1 }" >nul 2>&1
if %errorLevel% neq 0 (
    echo PowerShell 2.0 or newer is required.
    echo Please upgrade PowerShell and rerun this script.
    echo.
    pause
    goto menu
)

echo Starting configuration tests in PowerShell window...
echo.
start "" powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0utils\test zapret.ps1"
pause
goto menu

:: UPDATE AUTO_UPDATE.BAT ==============
:update_auto_update
cls
chcp 65001 > nul
powershell -Command "Write-Host 'Обновляю auto_update.bat с GitHub...' -ForegroundColor DarkMagenta"

set "GITHUB_URL=https://raw.githubusercontent.com/HolyLightRU/HolyZapret/main/auto_update.bat"
set "LOCAL_FILE=%~dp0auto_update.bat"

powershell -Command "try { Invoke-WebRequest -Uri '%GITHUB_URL%' -OutFile '%LOCAL_FILE%' -ErrorAction Stop; Write-Host 'Файл auto_update.bat успешно обновлён' -ForegroundColor Green } catch { Write-Host 'Ошибка при загрузке файла: $_' -ForegroundColor Red }"

pause
goto menu


:: GAME SWITCH ========================
:game_switch_status
chcp 437 > nul

set "gameFlagFile=%~dp0bin\game_filter.enabled"

if exist "%gameFlagFile%" (
    set "GameFilterStatus=enabled"
    set "GameFilter=1024-65535"
) else (
    set "GameFilterStatus=disabled"
    set "GameFilter=12"
)
exit /b

:game_switch
chcp 437 > nul
cls

if not exist "%gameFlagFile%" (
    echo Enabling game filter...
    echo ENABLED > "%gameFlagFile%"
    call :PrintYellow "Restart the zapret to apply the changes"
) else (
    echo Disabling game filter...
    del /f /q "%gameFlagFile%"
    call :PrintYellow "Restart the zapret to apply the changes"
)

pause
goto menu

:: STATUS ==============================
:service_status
cls
chcp 65001 > nul
for /f "tokens=2*" %%A in ('reg query "HKLM\System\CurrentControlSet\Services\zapret" /v zapret-discord-youtube 2^>nul') do echo Service strategy installed from "%%B"
call :test_service zapret
call :test_service WinDivert

tasklist /FI "IMAGENAME eq HolyZapret.exe" | find /I "HolyZapret.exe" > nul
if !errorlevel!==0 (
    call :PrintGreen "HolyZapret активирован. Удачного пользования!"
) else (
    call :PrintRed "HolyZapret не активирован"
)

pause
goto menu

:test_service
set "ServiceName=%~1"
set "ServiceStatus="

for /f "tokens=3 delims=: " %%A in ('sc query "%ServiceName%" ^| findstr /i "STATE"') do set "ServiceStatus=%%A"
set "ServiceStatus=%ServiceStatus: =%"

if "%ServiceStatus%"=="RUNNING" (
    if "%~2"=="soft" (
        echo "%ServiceName%" is ALREADY RUNNING as service, use "service.bat" and choose "Remove Services" first if you want to run standalone bat.
        pause
        exit /b
    ) else (
        echo "%ServiceName%" service is RUNNING.
    )
) else if not "%~2"=="soft" (
    echo "%ServiceName%" service is NOT running.
)

exit /b


:: REMOVE ==============================
:service_remove
cls
chcp 65001 > nul

set SRVCNAME=zapret
sc query "!SRVCNAME!" >nul 2>&1
if !errorlevel!==0 (
    net stop %SRVCNAME%
    sc delete %SRVCNAME%
    call :PrintGreen "Сервис HolyZapret / Zapret успешно удален."
) else (
    call :PrintYellow "Сервис HolyZapret / Zapret не установлен."
)

sc query "WinDivert" >nul 2>&1
if !errorlevel!==0 (
    net stop "WinDivert" >nul 2>&1
    sc delete "WinDivert" >nul 2>&1
)
net stop "WinDivert14" >nul 2>&1
sc delete "WinDivert14" >nul 2>&1

tasklist /FI "IMAGENAME eq HolyZapret.exe" | find /I "HolyZapret.exe" > nul
if !errorlevel!==0 (
    taskkill /IM winws.exe /F > nul
)

pause
goto menu


:: INSTALL =============================
:service_install
cls
chcp 65001 > nul

:: Main
cd /d "%~dp0"
set "BIN_PATH=%~dp0bin\"
set "LISTS_PATH=%~dp0lists\"
set "GameFilter=1024-65535"

:: Step 1: Choose folder (category)
echo ========================================
echo Шаг 1: Выберите категорию
echo ========================================
setlocal EnableDelayedExpansion
set "count=0"

:: Add root folder as option
set /a count+=1
echo !count!. Основные стратегии (корневая папка)
set "folder!count!=."

:: Find all subfolders with .bat files
for /d %%d in (*) do (
    dir "%%d\*.bat" /b >nul 2>&1
    if not errorlevel 1 (
        set /a count+=1
        echo !count!. %%d
        set "folder!count!=%%d"
    )
)

:: Choosing folder
set "folder_choice="
set /p "folder_choice=Выберите категорию (циферка): "
if "!folder_choice!"=="" goto menu

set "selectedFolder=!folder%folder_choice%!"
if not defined selectedFolder (
    echo Неверный выбор, возврат в меню...
    pause
    goto menu
)

:: Step 2: Choose strategy from selected folder
cls
echo ========================================
echo Шаг 2: Выберите стратегию из папки: !selectedFolder!
echo ========================================

set "count=0"
set "file_list="

:: Search in selected folder (root or subfolder)
if "!selectedFolder!"=="." (
    :: Search in root
    for %%f in (*.bat) do (
        set "filename=%%~nxf"
        set "lowername=!filename!"
        if /i not "!filename!"=="%~n0.bat" (
            if /i not "!lowername:~0,7!"=="service" if /i not "!lowername!"=="holy_host_system.bat" (
                set /a count+=1
                echo !count!. %%f
                set "file!count!=%%f"
                set "file_list=!file_list! %%f"
            )
        )
    )
) else (
    :: Search in subfolder
    for %%f in ("!selectedFolder!\*.bat") do (
        set "filename=%%~nxf"
        set "lowername=!filename!"
        if /i not "!lowername:~0,7!"=="service" if /i not "!lowername!"=="holy_host_system.bat" (
            set /a count+=1
            echo !count!. %%f
            set "file!count!=%%f"
            set "file_list=!file_list! %%f"
        )
    )
)

if !count!==0 (
    echo В выбранной папке не найдено подходящих .bat файлов!
    pause
    goto menu
)

:: Choosing file
set "choice="
set /p "choice=Выберите стратегию (циферка): "
if "!choice!"=="" goto menu

set "selectedFile=!file%choice%!"
if not defined selectedFile (
    echo Неверный выбор, возврат в меню...
    pause
    goto menu
)

echo.
echo Выбрана стратегия: !selectedFile!
echo.

:: Choose correct lists path for the selected strategy
set "LISTS_PATH=%~dp0lists\"
findstr /i "exp-list" "!selectedFile!" >nul 2>&1 && set "LISTS_PATH=%~dp0exp-list\"

:: Args that should be followed by value
set "args_with_value=sni host altorder dpi-desync-fake-tls-mod dpi-desync-hostfakesplit-mod"

:: Parsing args (mergeargs: 2=start param|3=arg with value|1=params args|0=default)
set "args="
set "capture=0"
set "mergeargs=0"
set QUOTE="

for /f "tokens=*" %%a in ('type "!selectedFile!"') do (
    set "line=%%a"

    echo !line! | findstr /i "%BIN%HolyZapret.exe" >nul
    if not errorlevel 1 (
        set "capture=1"
    )

    if !capture!==1 (
        if not defined args (
            set "line=!line:*%BIN%HolyZapret.exe"=!"
        )

        set "temp_args="
        set "temp_line=!line!"
        set "temp_line=!temp_line:*HolyZapret.exe"=!"

        set "temp_line=!temp_line:%%BIN%%=%BIN_PATH%!"
        set "temp_line=!temp_line:%%LISTS%%=%LISTS_PATH%!"
        set "temp_line=!temp_line:%%GameFilter%%=%GameFilter%!"

        set "args=!args! !temp_line!"

        if not "!temp_args!"=="" (
            set "args=!args! !temp_args!"
        )
    )
)

:: Creating service with parsed args
set ARGS=%args%
echo Итоговые аргументы: !ARGS!
set SRVCNAME=zapret

echo.
echo Устанавливаю сервис...

net stop %SRVCNAME% >nul 2>&1
sc delete %SRVCNAME% >nul 2>&1
sc create %SRVCNAME% binPath= "\"%BIN_PATH%HolyZapret.exe\" %ARGS%" DisplayName= "zapret" start= auto
sc description %SRVCNAME% "Zapret DPI bypass software"
sc start %SRVCNAME%

for %%F in ("!selectedFile!") do (
    set "filename=%%~nF"
)
reg add "HKLM\System\CurrentControlSet\Services\zapret" /v zapret-discord-youtube /t REG_SZ /d "!filename!" /f

powershell -Command "Write-Host 'Сервис успешно установлен!' -ForegroundColor Green"
echo.
powershell -Command "Write-Host '--- Проверка доступности сайтов после установки ---' -ForegroundColor Magenta"
call :check_sites

pause
goto menu


:: DIAGNOSTICS =========================
:service_diagnostics
chcp 65001 > nul
cls

:: AdguardSvc.exe
tasklist /FI "IMAGENAME eq AdguardSvc.exe" | find /I "AdguardSvc.exe" > nul
if !errorlevel!==0 (
    call :PrintRed "[X] Adguard process found. Adguard may cause problems with Discord"
    call :PrintRed "https://github.com/Flowseal/zapret-discord-youtube/issues/417"
) else (
    call :PrintGreen "Adguard check passed"
)
echo:

:: Killer
sc query | findstr /I "Killer" > nul
if !errorlevel!==0 (
    call :PrintRed "[X] Killer services found. Killer conflicts with zapret"
    call :PrintRed "https://github.com/Flowseal/zapret-discord-youtube/issues/2512#issuecomment-2821119513"
) else (
    call :PrintGreen "Killer check passed"
)
echo:

:: Intel Connectivity Network Service
sc query | findstr /I "Intel" | findstr /I "Connectivity" | findstr /I "Network" > nul
if !errorlevel!==0 (
    call :PrintRed "[X] Intel Connectivity Network Service found. It conflicts with zapret"
    call :PrintRed "https://github.com/ValdikSS/GoodbyeDPI/issues/541#issuecomment-2661670982"
) else (
    call :PrintGreen "Intel Connectivity check passed"
)
echo:

:: Check Point
set "checkpointFound=0"
sc query | findstr /I "TracSrvWrapper" > nul
if !errorlevel!==0 (
    set "checkpointFound=1"
)

sc query | findstr /I "EPWD" > nul
if !errorlevel!==0 (
    set "checkpointFound=1"
)

if !checkpointFound!==1 (
    call :PrintRed "[X] Check Point services found. Check Point conflicts with zapret"
    call :PrintRed "Try to uninstall Check Point"
) else (
    call :PrintGreen "Check Point check passed"
)
echo:

:: SmartByte
sc query | findstr /I "SmartByte" > nul
if !errorlevel!==0 (
    call :PrintRed "[X] SmartByte services found. SmartByte conflicts with zapret"
    call :PrintRed "Try to uninstall or disable SmartByte through services.msc"
) else (
    call :PrintGreen "SmartByte check passed"
)
echo:

:: VPN
sc query | findstr /I "VPN" > nul
if !errorlevel!==0 (
    call :PrintYellow "[?] Some VPN services found. Some VPNs can conflict with zapret"
    call :PrintYellow "Make sure that all VPNs are disabled"
) else (
    call :PrintGreen "VPN check passed"
)
echo:

:: DNS
set "dnsfound=0"
for /f "skip=1 tokens=*" %%a in ('wmic nicconfig where "IPEnabled=true" get DNSServerSearchOrder /format:table') do (
    echo %%a | findstr /i "192.168." >nul
    if !errorlevel!==0 (
        set "dnsfound=1"
    )
)
if !dnsfound!==1 (
    call :PrintYellow "[?] DNS servers are probably not specified."
    call :PrintYellow "Provider's DNS servers are automatically used, which may affect zapret. It is recommended to install well-known DNS servers and setup DoH"
) else (
    call :PrintGreen "DNS check passed"
)
echo:

:: Discord cache clearing
set "CHOICE="
set /p "CHOICE=Do you want to clear the Discord cache? (Y/N) (default: Y)  "
if "!CHOICE!"=="" set "CHOICE=Y"
if "!CHOICE!"=="y" set "CHOICE=Y"

if /i "!CHOICE!"=="Y" (
    tasklist /FI "IMAGENAME eq Discord.exe" | findstr /I "Discord.exe" > nul
    if !errorlevel!==0 (
        echo Discord is running, closing...
        taskkill /IM Discord.exe /F > nul
        if !errorlevel! == 0 (
            call :PrintGreen "Discord was successfully closed"
        ) else (
            call :PrintRed "Unable to close Discord"
        )
    )

    set "discordCacheDir=%appdata%\discord"

    for %%d in ("Cache" "Code Cache" "GPUCache") do (
        set "dirPath=!discordCacheDir!\%%~d"
        if exist "!dirPath!" (
            rd /s /q "!dirPath!"
            if !errorlevel!==0 (
                call :PrintGreen "Successfully deleted !dirPath!"
            ) else (
                call :PrintRed "Failed to delete !dirPath!"
            )
        ) else (
            call :PrintRed "!dirPath! does not exist"
        )
    )
)
echo:

pause
goto menu

:: CHECK SITES ============
:check_sites
cls
chcp 65001 > nul
powershell -Command "Write-Host '=== Проверка доступности сайтов ===' -ForegroundColor DarkMagenta"
echo:

set "LIST_FILE=%~dp0lists\check_list.txt"

if not exist "%LIST_FILE%" (
    call :PrintRed "[ERROR] Файл списка не найден!"
    echo Путь: %LIST_FILE%
    pause
    goto menu
)

set "PS_CMD="
set "PS_CMD=!PS_CMD! Add-Type -AssemblyName System.Net.Http; "
set "PS_CMD=!PS_CMD! $urls = Get-Content '%LIST_FILE%' | Where-Object { $_ -match '^http' }; "
set "PS_CMD=!PS_CMD! $client = New-Object System.Net.Http.HttpClient; "
set "PS_CMD=!PS_CMD! $client.Timeout = [TimeSpan]::FromSeconds(4); "
set "PS_CMD=!PS_CMD! $client.DefaultRequestHeaders.UserAgent.ParseAdd('Mozilla/5.0 (Windows NT 10.0; Win64; x64)'); "
set "PS_CMD=!PS_CMD! $taskMap = @{}; "
set "PS_CMD=!PS_CMD! $taskList = New-Object System.Collections.Generic.List[System.Threading.Tasks.Task]; "
set "PS_CMD=!PS_CMD! foreach ($url in $urls) { "
set "PS_CMD=!PS_CMD!     try { "
set "PS_CMD=!PS_CMD!         $t = $client.GetAsync($url, [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead); "
set "PS_CMD=!PS_CMD!         $taskMap[$t] = $url; "
set "PS_CMD=!PS_CMD!         $taskList.Add($t); "
set "PS_CMD=!PS_CMD!     } catch {} "
set "PS_CMD=!PS_CMD! } "
set "PS_CMD=!PS_CMD! Write-Host 'Запросы отправлены, ожидайте ответа...' -ForegroundColor Magenta; "
set "PS_CMD=!PS_CMD! try { [System.Threading.Tasks.Task]::WaitAll($taskList.ToArray()) } catch {} "
set "PS_CMD=!PS_CMD! $ok = 0; $total = 0; "
set "PS_CMD=!PS_CMD! foreach ($pair in $taskMap.GetEnumerator()) { "
set "PS_CMD=!PS_CMD!     $t = $pair.Key; "
set "PS_CMD=!PS_CMD!     $u = $pair.Value; "
set "PS_CMD=!PS_CMD!     $total++; "
set "PS_CMD=!PS_CMD!     $isSuccess = $false; "
set "PS_CMD=!PS_CMD!     if ($t.Status -eq 'RanToCompletion') { "
set "PS_CMD=!PS_CMD!         if ($t.Result.IsSuccessStatusCode) { $isSuccess = $true } "
set "PS_CMD=!PS_CMD!     } "
set "PS_CMD=!PS_CMD!     if ($isSuccess) { "
set "PS_CMD=!PS_CMD!         $ok++; Write-Host ' [OK]   ' -NoNewline -ForegroundColor Green; Write-Host $u -ForegroundColor White; "
set "PS_CMD=!PS_CMD!     } else { "
set "PS_CMD=!PS_CMD!         Write-Host ' [FAIL] ' -NoNewline -ForegroundColor Red; Write-Host $u -ForegroundColor DarkGray; "
set "PS_CMD=!PS_CMD!     } "
set "PS_CMD=!PS_CMD! } "
set "PS_CMD=!PS_CMD! Write-Host ''; "
set "PS_CMD=!PS_CMD! if ($ok -eq $total) { Write-Host ('Итог: Все сайты доступны (' + $ok + '/' + $total + ')') -ForegroundColor Green } "
set "PS_CMD=!PS_CMD! else { Write-Host ('Итог: Доступно ' + $ok + ' из ' + $total) -ForegroundColor Cyan } "

powershell -Command "!PS_CMD!"

echo:
pause
goto menu

:: Utility functions

:PrintGreen
powershell -Command "Write-Host \"%~1\" -ForegroundColor Green"
exit /b

:PrintRed
powershell -Command "Write-Host \"%~1\" -ForegroundColor Red"
exit /b

:PrintYellow
powershell -Command "Write-Host \"%~1\" -ForegroundColor Yellow"
exit /b