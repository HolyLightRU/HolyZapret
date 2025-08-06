@echo off
cd /d "%~dp0"
chcp 65001 > nul
title Прокидываю хосты

set "BIN=%~dp0bin\"
set "LISTS=%~dp0lists\"

NET SESSION >nul 2>&1
if %ERRORLEVEL% NEQ 0 (
    echo Запрос на получение административных прав...
    powershell -Command "Start-Process -FilePath '%~dpnx0' -Verb RunAs"
    exit /b
)
powershell -Command "Write-Host 'Обновляю записи в файле hosts...' -ForegroundColor Yellow"
set "HOSTS_FILE=C:\Windows\System32\drivers\etc\hosts"
set "HOSTS_LIST=%LISTS%hosts-list.txt"

powershell -Command "Copy-Item '%HOSTS_FILE%' '%HOSTS_FILE%.bak' -Force" >nul 2>&1

set "TEMP_FILE=%TEMP%\hosts.tmp"

for /f "tokens=*" %%a in ('type "%HOSTS_LIST%"') do (
    set "entry=%%a"
    set "domain="
    set "new_ip="

    for /f "tokens=1,2" %%i in ("%%a") do (
        set "new_ip=%%i"
        set "domain=%%j"
    )

    find /i "%domain%" "%HOSTS_FILE%" >nul 2>&1
    if !errorlevel! equ 0 (
        for /f "tokens=1,2" %%x in ('findstr /i "%domain%" "%HOSTS_FILE%"') do (
            set "old_ip=%%x"
        )
        if not "!old_ip!"=="!new_ip!" (
            powershell -Command "Write-Host 'Обновляю: !old_ip! !domain! -> !new_ip! !domain!' -ForegroundColor Cyan"
        )
    ) else (
        powershell -Command "Write-Host 'Добавлено: %%a' -ForegroundColor Magenta"
    )
    echo !new_ip! !domain!>> "%TEMP_FILE%"
)

for /f "tokens=*" %%z in ('type "%HOSTS_FILE%"') do (
    set "line=%%z"
    set "domain_in_line="
    for /f "tokens=2" %%d in ("%%z") do set "domain_in_line=%%d"
    
    if defined domain_in_line (
        findstr /i "!domain_in_line!" "%HOSTS_LIST%" >nul 2>&1
        if !errorlevel! neq 0 (
            echo %%z>> "%TEMP_FILE%"
        )
    ) else (
        echo %%z>> "%TEMP_FILE%"
    )
)

move /y "%TEMP_FILE%" "%HOSTS_FILE%" >nul 2>&1

:skip_hosts