@echo off
chcp 65001 > nul
set "LOCAL_VERSION=2.0.2"

:: External commands
if "%~1"=="status_zapret" (
    call :test_service zapret soft
    exit /b
)

if "%~1"=="check_updates" (
    if exist "%~dp0utils\check_updates.enabled" (
        if not "%~2"=="soft" (
            start /b "" "%~f0" check_updates soft
        ) else (
            call :service_check_updates soft
        )
    )
    exit /b
)

if "%~1"=="load_game_filter" (
    call :game_switch_status
    exit /b
)

if "%~1"=="load_user_lists" (
    call :load_user_lists
    exit /b
)

if "%1"=="admin" (
    echo Started with admin rights
) else (
    echo Requesting admin rights...
    powershell -Command "Start-Process 'cmd.exe' -ArgumentList '/c \"\"%~f0\" admin\"' -Verb RunAs"
    exit /b
)

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

setlocal EnableDelayedExpansion
chcp 65001 > nul
:menu
chcp 65001 > nul
cls
call :game_switch_status
powershell -Command "Write-Host 'HolyZapret %LOCAL_VERSION% Console' -ForegroundColor DarkMagenta; Write-Host '=======================' -ForegroundColor Magenta; Write-Host '1. Установить сервис (скрытая версия)' -ForegroundColor White; Write-Host '2. Удалить запрет и сервис' -ForegroundColor Cyan; Write-Host '3. Проверить текущий статус Запрета' -ForegroundColor White; Write-Host '4. Запустить диагностику' -ForegroundColor Magenta; Write-Host '5. Обновить hosts файл (holy_host_system)' -ForegroundColor DarkMagenta; Write-Host '6. Обновить auto_update.bat' -ForegroundColor White; Write-Host '7. Проверить доступность сайтов' -ForegroundColor Cyan; Write-Host '8. Проверить стратегии' -ForegroundColor Magenta; Write-Host '9. Локальный прокси для ТГ' -ForegroundColor DarkMagenta; Write-Host '0. Выход' -ForegroundColor White"

powershell -Command "Write-Host '10. Game Filter [!GameFilterStatus!]' -ForegroundColor White; Write-Host '11. Replace active fakes' -ForegroundColor Magenta"

set "menu_choice="
set /p "menu_choice=Выберите пункт (0-11): "

if "%menu_choice%"=="1" goto service_install
if "%menu_choice%"=="2" goto service_remove
if "%menu_choice%"=="3" goto service_status
if "%menu_choice%"=="4" goto service_diagnostics
if "%menu_choice%"=="5" goto run_hosts_system
if "%menu_choice%"=="6" goto update_auto_update
if "%menu_choice%"=="7" goto check_sites
if "%menu_choice%"=="8" goto run_tests
if "%menu_choice%"=="9" goto run_tg_ws_proxy
if "%menu_choice%"=="10" goto game_switch
if "%menu_choice%"=="11" goto replace_active_fakes
if "%menu_choice%"=="0" exit /b
goto menu

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

:run_tests
chcp 65001 >nul
cls

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

:update_auto_update
cls
chcp 65001 > nul
powershell -Command "Write-Host 'Обновляю auto_update.bat с GitHub...' -ForegroundColor DarkMagenta"

set "GITHUB_URL=https://raw.githubusercontent.com/HolyLightRU/HolyZapret/main/auto_update.bat"
set "LOCAL_FILE=%~dp0auto_update.bat"

powershell -Command "try { Invoke-WebRequest -Uri '%GITHUB_URL%' -OutFile '%LOCAL_FILE%' -ErrorAction Stop; Write-Host 'Файл auto_update.bat успешно обновлён' -ForegroundColor Green } catch { Write-Host 'Ошибка при загрузке файла: $_' -ForegroundColor Red }"

pause
goto menu

:load_user_lists
set "LISTS_PATH=%~dp0exp-list\"

if not exist "%LISTS_PATH%ipset-exclude-user.txt" (
    echo 203.0.113.113/32>"%LISTS_PATH%ipset-exclude-user.txt"
)
if not exist "%LISTS_PATH%list-general-user.txt" (
    echo # Never leave this file empty>"%LISTS_PATH%list-general-user.txt"
    echo domain.example.abc>>"%LISTS_PATH%list-general-user.txt"
)
if not exist "%LISTS_PATH%list-exclude-user.txt" (
    echo domain.example.abc>"%LISTS_PATH%list-exclude-user.txt"
)

for %%F in ("%LISTS_PATH%ipset-exclude-user.txt") do if %%~zF==0 echo 203.0.113.113/32>"%LISTS_PATH%ipset-exclude-user.txt"
for %%F in ("%LISTS_PATH%list-general-user.txt") do if %%~zF==0 (
    echo # Never leave this file empty>"%LISTS_PATH%list-general-user.txt"
    echo domain.example.abc>>"%LISTS_PATH%list-general-user.txt"
)
for %%F in ("%LISTS_PATH%list-exclude-user.txt") do if %%~zF==0 echo domain.example.abc>"%LISTS_PATH%list-exclude-user.txt"
exit /b

:game_switch_status
set "gameFlagFile=%~dp0bin\game_filter.enabled"
set "GameFilterMode=disabled"
set "GameFilterTCPRange=1024-65535"
set "GameFilterUDPRange=1024-65535"
set "GameFilterStatus=disabled"
set "GameFilter=12"
set "GameFilterTCP=12"
set "GameFilterUDP=12"

if not exist "%gameFlagFile%" exit /b

set "GameFilterTCPCandidate="
set "GameFilterUDPCandidate="
for /f "usebackq tokens=1,* delims== " %%A in ("%gameFlagFile%") do (
    if /i "%%A"=="mode" set "GameFilterMode=%%B"
    if /i "%%A"=="all" set "GameFilterMode=all"
    if /i "%%A"=="udp" (
        if "%%B"=="" (set "GameFilterMode=udp") else set "GameFilterUDPCandidate=%%B"
    )
    if /i "%%A"=="tcp" (
        if "%%B"=="" (set "GameFilterMode=tcp") else set "GameFilterTCPCandidate=%%B"
    )
    if /i "%%A"=="ENABLED" set "GameFilterMode=all"
)

call :validate_game_filter_range "%GameFilterTCPCandidate%"
if defined ValidatedGameFilterRange set "GameFilterTCPRange=%ValidatedGameFilterRange%"
call :validate_game_filter_range "%GameFilterUDPCandidate%"
if defined ValidatedGameFilterRange set "GameFilterUDPRange=%ValidatedGameFilterRange%"

if /i "%GameFilterMode%"=="all" (
    set "GameFilterStatus=enabled (TCP and UDP)"
    set "GameFilter=%GameFilterTCPRange%"
    set "GameFilterTCP=%GameFilterTCPRange%"
    set "GameFilterUDP=%GameFilterUDPRange%"
) else if /i "%GameFilterMode%"=="tcp" (
    set "GameFilterStatus=enabled (TCP)"
    set "GameFilter=%GameFilterTCPRange%"
    set "GameFilterTCP=%GameFilterTCPRange%"
    set "GameFilterUDP=12"
) else if /i "%GameFilterMode%"=="udp" (
    set "GameFilterStatus=enabled (UDP)"
    set "GameFilter=%GameFilterUDPRange%"
    set "GameFilterTCP=12"
    set "GameFilterUDP=%GameFilterUDPRange%"
) else (
    set "GameFilterMode=disabled"
)
exit /b

:game_switch
cls
call :game_switch_status

echo Select game filter option:
if "%GameFilterMode%"=="disabled"   (echo   1. * Disable) else      echo   1.   Disable
if "%GameFilterMode%"=="all"        (echo   2. * TCP and UDP) else  echo   2.   TCP and UDP
if "%GameFilterMode%"=="tcp"        (echo   3. * TCP) else          echo   3.   TCP
if "%GameFilterMode%"=="udp"        (echo   4. * UDP) else          echo   4.   UDP
echo.
echo   5. Change TCP port range (current: %GameFilterTCPRange%)
echo   6. Change UDP port range (current: %GameFilterUDPRange%)
echo   7. Change TCP and UDP port ranges
echo.
echo   0. Exit
echo.
set "GameFilterChoice=0"
set /p "GameFilterChoice=Select option (0-7, default: 0): "
if "%GameFilterChoice%"=="" set "GameFilterChoice=0"

if "%GameFilterChoice%"=="1" (
    set "GameFilterMode=disabled"
) else if "%GameFilterChoice%"=="2" (
    set "GameFilterMode=all"
) else if "%GameFilterChoice%"=="3" (
    set "GameFilterMode=tcp"
) else if "%GameFilterChoice%"=="4" (
    set "GameFilterMode=udp"
) else if "%GameFilterChoice%"=="5" (
    call :change_game_filter_range tcp
) else if "%GameFilterChoice%"=="6" (
    call :change_game_filter_range udp
) else if "%GameFilterChoice%"=="7" (
    call :change_game_filter_range all
) else (
    goto menu
)

echo.
call :save_game_filter_settings
call :PrintYellow "Restart the zapret to apply the changes"
pause
goto game_switch

:change_game_filter_range
set "GameFilterRangeInput="

echo.
echo Changing ports for %~1 (example: 1024-1934,1936-65535, default: 1024-65535)
set /p "GameFilterRangeInput=Enter ports/ranges: "
call :validate_game_filter_range "%GameFilterRangeInput%"
if not defined ValidatedGameFilterRange (
    call :PrintRed "Invalid input. Please enter valid ports/ranges."
    pause
    goto game_switch
)

if /i "%~1"=="tcp" set "GameFilterTCPRange=%ValidatedGameFilterRange%"
if /i "%~1"=="udp" set "GameFilterUDPRange=%ValidatedGameFilterRange%"
if /i "%~1"=="all" (
    set "GameFilterTCPRange=%ValidatedGameFilterRange%"
    set "GameFilterUDPRange=%ValidatedGameFilterRange%"
)
exit /b

:validate_game_filter_range
set "ValidatedGameFilterRange="
setlocal EnableDelayedExpansion
set "GameFilterRangeToValidate=%~1"
set "GameFilterRangeToValidate=!GameFilterRangeToValidate: =!"
if not defined GameFilterRangeToValidate exit /b
for %%A in ("!GameFilterRangeToValidate:,=" "!") do (
    call :gf_validate_item "%%~A"
    if not defined GameFilterRangeItemValid exit /b
)
endlocal & set "ValidatedGameFilterRange=%GameFilterRangeToValidate%"
exit /b

:gf_validate_item
set "GameFilterRangeItemValid="
setlocal EnableDelayedExpansion
set "GameFilterRangeItem=%~1"

echo(!GameFilterRangeItem!| findstr /r /x /c:"[1-9][0-9]*" /c:"[1-9][0-9]*-[1-9][0-9]*" > nul || exit /b
for /f "tokens=1,2 delims=-" %%A in ("!GameFilterRangeItem!") do (
    set "GameFilterRangeStart=%%A"
    set "GameFilterRangeEnd=%%B"
)
if not defined GameFilterRangeEnd set "GameFilterRangeEnd=!GameFilterRangeStart!"

if not "!GameFilterRangeStart:~5,1!"=="" exit /b
if not "!GameFilterRangeEnd:~5,1!"=="" exit /b
set /a GameFilterRangeStartNumber=GameFilterRangeStart, GameFilterRangeEndNumber=GameFilterRangeEnd
if !GameFilterRangeStartNumber! gtr 65535 exit /b
if !GameFilterRangeEndNumber! gtr 65535 exit /b
if !GameFilterRangeStartNumber! gtr !GameFilterRangeEndNumber! exit /b

endlocal & set "GameFilterRangeItemValid=1"
exit /b

:save_game_filter_settings
>"%gameFlagFile%" (
    echo mode=%GameFilterMode%
    echo tcp=%GameFilterTCPRange%
    echo udp=%GameFilterUDPRange%
)
exit /b

:replace_active_fakes
cls

set "BIN_PATH=%~dp0bin\"
set "fake_count=0"
set "fake_type="
set "fake_number="
set "discord_hash="
set "game_hash="
set "current_discord_fake=(not found)"
set "current_game_fake=(not found)"

if not exist "%BIN_PATH%" (
    echo Error: bin folder not found.
    pause
    goto menu
)

pushd "%BIN_PATH%"
for /f "tokens=1,2,3 delims=|" %%A in ('powershell -NoProfile -Command "foreach ($item in @(@{Name='ACTIVE_DISCORD_UDP.bin'; Label='ACTIVE_DISCORD'},@{Name='ACTIVE_GAME_UDP.bin'; Label='ACTIVE_GAME'})) { if (Test-Path -LiteralPath $item.Name) { Write-Output ($item.Label + [char]124 + $item.Label + [char]124 + (Get-FileHash -LiteralPath $item.Name -Algorithm SHA256).Hash) } }; $files = @(Get-ChildItem -LiteralPath . -File -Filter '*.bin'); foreach ($file in $files) { if ($file.BaseName -notlike 'ACTIVE_*') { Write-Output ('FAKE' + [char]124 + $file.BaseName + [char]124 + (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash) } }"') do (
    if "%%A"=="ACTIVE_DISCORD" (
        set "discord_hash=%%C"
    ) else if "%%A"=="ACTIVE_GAME" (
        set "game_hash=%%C"
    ) else if "%%A"=="FAKE" (
        set /a fake_count+=1
        set "fake_file!fake_count!=%BIN_PATH%%%B.bin"
        set "fake_name!fake_count!=%%B"
        set "fake_hash!fake_count!=%%C"
    )
)
popd

if !fake_count! EQU 0 (
    echo No .bin files were found in the bin folder.
    pause
    goto menu
)

for /l %%N in (1,1,!fake_count!) do (
    if defined discord_hash if /i "!fake_hash%%N!"=="!discord_hash!" set "current_discord_fake=!fake_name%%N!"
    if defined game_hash if /i "!fake_hash%%N!"=="!game_hash!" set "current_game_fake=!fake_name%%N!"
)

:replace_active_fakes_prompt
echo.
echo Enter the fake type number and the fake file number to replace it with.
echo Example: 1 4 (replaces Discord UDP with fake file under number 4)
echo          2 1 (replaces GameFilter UDP with fake file under number 1)
echo.
echo Press ENTER or 0 to return.
echo.
echo   ----------------------------------------
echo.
echo Fake types:
echo   1. Discord UDP     (current: !current_discord_fake!)
echo   2. GameFilter UDP  (current: !current_game_fake!)
echo.
echo Fake files:
for /l %%N in (1,1,!fake_count!) do echo   %%N. !fake_name%%N!
echo.

set "replace_choice="
set /p "replace_choice=Enter choice: "
if not defined replace_choice goto menu
if "!replace_choice!"=="0" goto menu

set "active_file="
set "fake_type="
set "fake_number="
for /f "tokens=1,2" %%A in ("!replace_choice!") do (
    set "fake_type=%%A"
    set "fake_number=%%B"
)

if "!fake_type!"=="1" (
    set "active_file=%BIN_PATH%ACTIVE_DISCORD_UDP.bin"
) else if "!fake_type!"=="2" (
    set "active_file=%BIN_PATH%ACTIVE_GAME_UDP.bin"
) else (
    echo Invalid fake type.
    pause
    cls
    goto replace_active_fakes_prompt
)

set "source_file="
for /l %%N in (1,1,!fake_count!) do if "%%N"=="!fake_number!" set "source_file=!fake_file%%N!"
if not defined source_file (
    echo Invalid fake file number.
    pause
    cls
    goto replace_active_fakes_prompt
)

del /f /q "!active_file!" >nul 2>&1
copy /y "!source_file!" "!active_file!" >nul
if errorlevel 1 (
    echo Failed to replace the active fake file.
) else (
    echo Active fake file replaced successfully.
    for /l %%N in (1,1,!fake_count!) do if "%%N"=="!fake_number!" (
        if "!fake_type!"=="1" set "current_discord_fake=!fake_name%%N!"
        if "!fake_type!"=="2" set "current_game_fake=!fake_name%%N!"
    )
)
pause
cls
goto replace_active_fakes_prompt

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
    taskkill /IM HolyZapret.exe /F > nul
)

tasklist /FI "IMAGENAME eq winws.exe" | find /I "winws.exe" > nul
if !errorlevel!==0 (
    taskkill /IM winws.exe /F > nul
)

pause
goto menu

:service_install
cls
chcp 65001 > nul

cd /d "%~dp0"
set "BIN_PATH=%~dp0bin\"
set "LISTS_PATH=%~dp0lists\"
set "GameFilter=1024-65535"

echo ========================================
echo Шаг 1: Выберите категорию
echo ========================================
setlocal EnableDelayedExpansion
set "count=0"

set /a count+=1
echo !count!. Основные стратегии (корневая папка)
set "folder!count!=."

for /d %%d in (*) do (
    dir "%%d\*.bat" /b >nul 2>&1
    if not errorlevel 1 (
        set /a count+=1
        echo !count!. %%d
        set "folder!count!=%%d"
    )
)

set "folder_choice="
set /p "folder_choice=Выберите категорию (циферка): "
if "!folder_choice!"=="" goto menu

set "selectedFolder=!folder%folder_choice%!"
if not defined selectedFolder (
    echo Неверный выбор, возврат в меню...
    pause
    goto menu
)

cls
echo ========================================
echo Шаг 2: Выберите стратегию из папки: !selectedFolder!
echo ========================================

set "count=0"
set "file_list="

if "!selectedFolder!"=="." (
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

set "LISTS_PATH=%~dp0lists\"
findstr /i "exp-list" "!selectedFile!" >nul 2>&1 && set "LISTS_PATH=%~dp0exp-list\"

set "args_with_value=sni host altorder dpi-desync-fake-tls-mod dpi-desync-hostfakesplit-mod"

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
reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "HolyTGProxy" >nul 2>&1
if !errorlevel! neq 0 (
    powershell -Command "Write-Host 'Прокси для Telegram еще не настроен.' -ForegroundColor Cyan"
    set /p "goto_tg=Перейти к настройке TG WS Proxy? (Y/N): "
    if /i "!goto_tg!"=="Y" (
        goto run_tg_ws_proxy
    )
)

echo.
powershell -Command "Write-Host '--- Проверка доступности сайтов после установки ---' -ForegroundColor Magenta"
call :check_sites

pause
goto menu

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

sc query | findstr /I "Intel" | findstr /I "Connectivity" | findstr /I "Network" > nul
if !errorlevel!==0 (
    call :PrintRed "[X] Intel Connectivity Network Service found. It conflicts with zapret"
    call :PrintRed "https://github.com/ValdikSS/GoodbyeDPI/issues/541#issuecomment-2661670982"
) else (
    call :PrintGreen "Intel Connectivity check passed"
)
echo:

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

sc query | findstr /I "SmartByte" > nul
if !errorlevel!==0 (
    call :PrintRed "[X] SmartByte services found. SmartByte conflicts with zapret"
    call :PrintRed "Try to uninstall or disable SmartByte through services.msc"
) else (
    call :PrintGreen "SmartByte check passed"
)
echo:

sc query | findstr /I "VPN" > nul
if !errorlevel!==0 (
    call :PrintYellow "[?] Some VPN services found. Some VPNs can conflict with zapret"
    call :PrintYellow "Make sure that all VPNs are disabled"
) else (
    call :PrintGreen "VPN check passed"
)
echo:

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
tasklist /FI "IMAGENAME eq tg-ws-proxy.exe" | find /I "tg-ws-proxy.exe" > nul
if !errorlevel!==0 (
    call :PrintGreen "TG WS Proxy: активен"
) else (
    call :PrintYellow "TG WS Proxy: не запущен"
)
pause
goto menu

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

:run_tg_ws_proxy
cls
chcp 65001 > nul
echo ========================================
echo Telegram WS Proxy Manager
echo ========================================
echo 1. Запустить прокси
echo 2. Остановить прокси
echo 3. Проверить статус
echo 4. Включить автозапуск (при старте Windows)
echo 5. Отключить автозапуск
echo 0. Назад
set /p "tg_choice=Выбор: "

if "!tg_choice!"=="1" (
    tasklist /FI "IMAGENAME eq tg-ws-proxy.exe" | find /I "tg-ws-proxy.exe" > nul
    if !errorlevel!==0 (
        call :PrintYellow "TG WS Proxy уже запущен."
    ) else (
        start "" /B "%~dp0bin\tg-ws-proxy.exe" --port 1443 --no-gui
        call :PrintGreen "TG WS Proxy успешно запущен на порту 1443."
    )
    pause
    goto run_tg_ws_proxy
)
if "!tg_choice!"=="2" (
    taskkill /IM tg-ws-proxy.exe /F >nul 2>&1
    if !errorlevel!==0 (
        call :PrintYellow "Процесс tg-ws-proxy остановлен."
    ) else (
        call :PrintRed "Процесс не найден или уже остановлен."
    )
    reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "HolyTGProxy" /f >nul 2>&1
        call :PrintYellow "TG WS Proxy удален из автозагрузки."
        pause
        goto run_tg_ws_proxy
)
if "!tg_choice!"=="3" (
    echo.
    tasklist /FI "IMAGENAME eq tg-ws-proxy.exe" | find /I "tg-ws-proxy.exe" > nul
    if !errorlevel!==0 (
        call :PrintGreen "Статус процесса: АКТИВЕН"
        netstat -ano | findstr :1443
    ) else (
        call :PrintRed "Статус процесса: НЕ ЗАПУЩЕН"
    )

    reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "HolyTGProxy" >nul 2>&1
    if !errorlevel!==0 (
        call :PrintGreen "Автозагрузка: ВКЛЮЧЕНА"
    ) else (
        call :PrintYellow "Автозагрузка: ВЫКЛЮЧЕНА"
    )
    echo.
    pause
    goto run_tg_ws_proxy
)
if "!tg_choice!"=="4" (
    reg add "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "HolyTGProxy" /t REG_SZ ^
        /d "cmd.exe /c start \"\" /B /D \"%~dp0bin\" \"%~dp0bin\tg-ws-proxy.exe\" --port 1443 --no-gui" /f >nul
    call :PrintGreen "TG WS Proxy добавлен в автозагрузку Windows."
    pause
    goto run_tg_ws_proxy
)
if "!tg_choice!"=="5" (
    reg delete "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v "HolyTGProxy" /f >nul 2>&1
    call :PrintYellow "TG WS Proxy удален из автозагрузки."
    pause
    goto run_tg_ws_proxy
)
if "!tg_choice!"=="0" goto menu
goto run_tg_ws_proxy

:service_check_updates
chcp 65001 > nul
powershell -Command "Write-Host 'Проверка обновлений HolyZapret выполняется при запуске manager.bat.' -ForegroundColor Cyan"
if not "%~1"=="soft" pause
exit /b

:PrintGreen
powershell -Command "Write-Host \"%~1\" -ForegroundColor Green"
exit /b

:PrintRed
powershell -Command "Write-Host \"%~1\" -ForegroundColor Red"
exit /b

:PrintYellow
powershell -Command "Write-Host \"%~1\" -ForegroundColor Yellow"
exit /b