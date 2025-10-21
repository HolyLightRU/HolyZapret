@echo off
set "LOCAL_VERSION=1.3.6"

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



:: MENU ================================
@echo off
setlocal EnableDelayedExpansion
chcp 65001 > nul
:menu
cls
powershell -Command "Write-Host 'HolyZapret 1.3.6 Console' -ForegroundColor DarkMagenta; Write-Host '=======================' -ForegroundColor Magenta; Write-Host '1. Установить сервис (скрытая версия)' -ForegroundColor White; Write-Host '2. Удалить запрет и сервис' -ForegroundColor Cyan; Write-Host '3. Проверить текущий статус Запрета' -ForegroundColor White; Write-Host '4. Запустить диагностику' -ForegroundColor Magenta; Write-Host '5. Обновить hosts файл (holy_host_system)' -ForegroundColor DarkMagenta; Write-Host '6. Обновить auto_update.bat' -ForegroundColor White; Write-Host '0. Выход' -ForegroundColor White"

set "menu_choice="
set /p "menu_choice=Выберите пункт (0-6): "

if "%menu_choice%"=="1" goto service_install
if "%menu_choice%"=="2" goto service_remove
if "%menu_choice%"=="3" goto service_status
if "%menu_choice%"=="4" goto service_diagnostics
if "%menu_choice%"=="5" goto run_hosts_system
if "%menu_choice%"=="6" goto update_auto_update
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
net stop %SRVCNAME%
sc delete %SRVCNAME%

net stop "WinDivert"
sc delete "WinDivert"
net stop "WinDivert14"
sc delete "WinDivert14"

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

:: Searching for .bat files in current folder and additional\, except files that start with "service"
echo Выберите одну стратегию из списка:
setlocal EnableDelayedExpansion
set "count=0"

:: Поиск в корне
for %%f in (*.bat) do (
    set "filename=%%~nxf"
    set "lowername=!filename!"
    if /i not "!lowername:~0,7!"=="service" if /i not "!lowername!"=="holy_host_system.bat" (
        set /a count+=1
        echo !count!. %%f
        set "file!count!=%%f"
    )
)

:: Поиск в additional\
for %%f in (additional\*.bat) do (
    set "filename=%%~nxf"
    set "lowername=!filename!"
    if /i not "!lowername:~0,7!"=="service" if /i not "!lowername!"=="holy_host_system.bat" (
        set /a count+=1
        echo !count!. %%f
        set "file!count!=%%f"
    )
)

:: Choosing file
set "choice="
set /p "choice=Выберите индекс необходимой стратегии (циферка): "
if "!choice!"=="" goto :eof

set "selectedFile=!file%choice%!"
if not defined selectedFile (
    echo Invalid choice, exiting...
    pause
    goto menu
)


:: Args that should be followed by value
set "args_with_value=sni"

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
        for %%i in (!line!) do (
            set "arg=%%i"

            if not "!arg!"=="^" (
                if "!arg:~0,2!" EQU "--" if not !mergeargs!==0 (
                    set "mergeargs=0"
                )

                if "!arg:~0,1!" EQU "!QUOTE!" (
                    set "arg=!arg:~1,-1!"

                    echo !arg! | findstr ":" >nul
                    if !errorlevel!==0 (
                        set "arg=\!QUOTE!!arg!\!QUOTE!"
                    ) else if "!arg:~0,1!"=="@" (
                        set "arg=\!QUOTE!@%~dp0!arg:~1!\!QUOTE!"
                    ) else if "!arg:~0,5!"=="%%BIN%%" (
                        set "arg=\!QUOTE!!BIN_PATH!!arg:~5!\!QUOTE!"
                    ) else if "!arg:~0,7!"=="%%LISTS%%" (
                        set "arg=\!QUOTE!!LISTS_PATH!!arg:~7!\!QUOTE!"
                    ) else (
                        set "arg=\!QUOTE!%~dp0!arg!\!QUOTE!"
                    )
                )

                if !mergeargs!==1 (
                    set "temp_args=!temp_args!,!arg!"
                ) else if !mergeargs!==3 (
                    set "temp_args=!temp_args!=!arg!"
                    set "mergeargs=1"
                ) else (
                    set "temp_args=!temp_args! !arg!"
                )

                if "!arg:~0,2!" EQU "--" (
                    set "mergeargs=2"
                ) else if !mergeargs!==2 (
                    set "mergeargs=1"
                ) else if !mergeargs!==1 (
                    for %%x in (!args_with_value!) do (
                        if /i "%%x"=="!arg!" (
                            set "mergeargs=3"
                        )
                    )
                )
            )
        )

        if not "!temp_args!"=="" (
            set "args=!args! !temp_args!"
        )
    )
)

:: Creating service with parsed args
set ARGS=%args%
echo Final args: !ARGS!
set SRVCNAME=zapret

net stop %SRVCNAME% >nul 2>&1
sc delete %SRVCNAME% >nul 2>&1
sc create %SRVCNAME% binPath= "\"%BIN_PATH%HolyZapret.exe\" %ARGS%" DisplayName= "zapret" start= auto
sc description %SRVCNAME% "Zapret DPI bypass software"
sc start %SRVCNAME%
for %%F in ("!file%choice%!") do (
    set "filename=%%~nF"
)
reg add "HKLM\System\CurrentControlSet\Services\zapret" /v zapret-discord-youtube /t REG_SZ /d "!filename!" /f

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